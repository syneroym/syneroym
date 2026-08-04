//! Member master keys, held in the supervisor's own encrypted service vault
//! rather than on disk beside a config (ADR-0020 §4). Keyed by the same
//! computable name A0 uses for files (`member-<instance>-<service>-<index>`),
//! so adoption is a read.
//!
//! **Also holds the app instance's own master** (M05A A7, ADR-0022 §1),
//! minted at `adopt` under `app-<app_instance_id>` beside the member
//! masters it names. It delegates nothing and signs nothing in this slice
//! -- it exists so the app has a network identity that outlives the
//! supervisor managing it, for a later slice's registry publication to use.
//!
//! **Local by construction.** The supervisor writes to *its own* node's
//! service database through `StorageProvider::open_service_db`, an
//! in-process call -- it never issues a remote `security/set-secret` and so
//! never needs `substrate/admin` on a managed substrate. That is what keeps
//! failure-matrix row 14's blast-radius claim intact and what resolves the
//! standing P0 backlog row on this exact question.
//!
//! **No master ever crosses the wire.** `export_master`/`import_master` move
//! a *file* on the supervisor's own host, into and out of the operator-
//! declared backup directory -- `export` returns a path, `import` takes a
//! name, and neither carries key bytes.

use std::{
    collections::BTreeMap,
    error, fmt, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use syneroym_app_orchestration::models::{DeploymentPlan, ServiceId};
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate};

/// Marker substring `SqliteStorageProvider::verify_encryption_mode` returns
/// when encryption is on and no KEK has been injected -- distinguishes a
/// locked vault from a genuine storage failure.
const ENCRYPTION_KEY_REQUIRED: &str = "EncryptionKeyRequired";

#[derive(Debug)]
pub enum VaultError {
    /// The vault's `KeyStore` holds no KEK -- the ordinary state of a
    /// freshly-booted supervisor, since the KEK arrives by
    /// `security.inject-kek` and does not survive a restart. Every caller
    /// distinguishes it from a real storage failure, because the operator
    /// action differs.
    Locked,
    Storage(anyhow::Error),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => write!(
                f,
                "this supervisor's vault is locked (no KEK injected); run: roymctl --substrate \
                 <this node> security inject-kek --kek-hex <...>"
            ),
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl error::Error for VaultError {}

impl From<anyhow::Error> for VaultError {
    fn from(e: anyhow::Error) -> Self {
        if e.to_string().contains(ENCRYPTION_KEY_REQUIRED) {
            Self::Locked
        } else {
            Self::Storage(e)
        }
    }
}

fn validate_backup_name(name: &str) -> Result<(), VaultError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || Path::new(name).is_absolute()
    {
        return Err(VaultError::Storage(anyhow::anyhow!(
            "invalid master name '{name}': must be a bare file stem, no path separators or '..'"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_backup_dir(dir: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    // `create` errors `AlreadyExists` when the directory is already there;
    // that is success, not a failure to report.
    match builder.create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(VaultError::Storage(e.into())),
    }
}

#[cfg(not(unix))]
fn ensure_backup_dir(dir: &Path) -> Result<(), VaultError> {
    fs::create_dir_all(dir).map_err(|e| VaultError::Storage(e.into()))
}

/// Which kind of master `get_or_mint` is minting -- read only to pick the
/// mint-warning's wording (M05A A7, D-A7-9). A member master's loss story
/// ("orphans every row this member has written") is the wrong noun and the
/// wrong consequence for an app master, which owns no rows and instead
/// loses the app instance's own network identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterKind {
    Member,
    App,
}

/// One minted or resolved member master, named by its logical service and
/// carrying the DID an operator should record and back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedMaster {
    pub service_name: String,
    pub master_did: String,
    /// The name `export_master`/`import_master` actually key on
    /// (`member_master_name`'s output) -- distinct from `service_name`,
    /// the bare logical name, which is not a valid vault key (S1, Slice
    /// A5b review).
    pub vault_name: String,
    /// This member's ordinal (M05A A5e, D-A5e-2/§33.17): `submit` returns
    /// N rows per scaled service that are otherwise identical --
    /// `service_name` alone cannot distinguish them, only `vault_name`
    /// can, and an operator reading the printed line should not have to
    /// parse it back out of that.
    pub member_index: u32,
}

pub struct MasterVault {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    /// The fixed scope this vault's encrypted service database opens
    /// under -- never caller-supplied, so the vault cannot be redirected.
    service_id: String,
    /// Where `export_master`/`import_master` read and write files --
    /// operator-declared via `SupervisorRole::master_backup_dir`, never
    /// caller-supplied.
    backup_dir: PathBuf,
    /// Serializes `get_or_mint`'s read-then-write across concurrent
    /// callers. `SupervisorService::dispatch` takes `&self`, so two
    /// `submit`s for the same instance can otherwise interleave: both see
    /// no existing master, both mint one, and the later `write_secret`
    /// wins -- the earlier call has already certified and deployed a
    /// member under a master no longer in the vault, which ADR-0020 §4
    /// calls unrecoverable (S3, Slice A5b review). One process-wide lock
    /// is enough: minting is rare and never on a hot path.
    mint_lock: tokio::sync::Mutex<()>,
}

impl fmt::Debug for MasterVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterVault")
            .field("service_id", &self.service_id)
            .field("backup_dir", &self.backup_dir)
            .finish_non_exhaustive()
    }
}

impl MasterVault {
    pub fn new(
        storage_provider: Arc<dyn StorageProvider>,
        key_store: Arc<KeyStore>,
        service_id: String,
        backup_dir: PathBuf,
    ) -> Self {
        Self {
            storage_provider,
            key_store,
            service_id,
            backup_dir,
            mint_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Whether a KEK has been injected, so vault reads can succeed at all.
    /// A cheap, non-`Result`, no-I/O check the resident loop makes once per
    /// pass before it attempts any renewal work: a locked vault is the
    /// ordinary state of a freshly-booted supervisor, and finding that out
    /// by failing one `get_or_mint` per member is both slower and noisier
    /// than asking up front. `VaultError::Locked` remains the per-call
    /// answer for the (defensive) case where the vault is locked between
    /// this check and the call.
    #[must_use]
    pub fn kek_is_loaded(&self) -> bool {
        self.key_store.kek_is_loaded()
    }

    async fn open(&self) -> Result<Box<dyn syneroym_data_db::traits::ServiceStore>, VaultError> {
        self.storage_provider
            .open_service_db(&self.service_id, &self.key_store)
            .await
            .map_err(VaultError::from)
    }

    /// Reads the named master, or `Ok(None)` if it has never been minted or
    /// imported.
    pub async fn get(&self, name: &str) -> Result<Option<Identity>, VaultError> {
        let store = self.open().await?;
        let bytes = store.reveal_secret(name).await.map_err(VaultError::Storage)?;
        let Some(bytes) = bytes else { return Ok(None) };
        let array: [u8; 32] = bytes.try_into().map_err(|_| {
            VaultError::Storage(anyhow::anyhow!(
                "vault entry '{name}' is not a valid 32-byte identity seed"
            ))
        })?;
        Ok(Some(Identity::from_bytes(&array)))
    }

    async fn store_bytes(&self, name: &str, bytes: &[u8; 32]) -> Result<(), VaultError> {
        let store = self.open().await?;
        store.write_secret(name, bytes).await.map_err(VaultError::Storage)
    }

    /// Mints and stores if absent -- the ordinary path. Reuses an existing
    /// master rather than minting a second one for the same name. `kind`
    /// (M05A A7, D-A7-9) picks only the mint-warning's wording below.
    pub async fn get_or_mint(&self, name: &str, kind: MasterKind) -> Result<Identity, VaultError> {
        // Held across the whole read-then-write below (`mint_lock`'s own
        // doc): without it, two concurrent callers for the same `name`
        // both pass the `get` check and both mint.
        let _guard = self.mint_lock.lock().await;
        if let Some(existing) = self.get(name).await? {
            return Ok(existing);
        }
        let identity = Identity::generate().map_err(VaultError::Storage)?;
        self.store_bytes(name, &identity.to_bytes()).await?;
        let did = substrate::derive_did_key(&identity.public_key());
        match kind {
            MasterKind::Member => tracing::warn!(
                name,
                did,
                "minted a new member master into the supervisor vault -- back it up with `roymctl \
                 supervisor export-master {name}`; losing it is unrecoverable and orphans every \
                 row this member has written"
            ),
            MasterKind::App => tracing::warn!(
                name,
                did,
                "minted a new app-instance master into the supervisor vault -- back it up with \
                 `roymctl supervisor export-master {name}`; losing it is unrecoverable and loses \
                 this app instance's own network identity"
            ),
        }
        Ok(identity)
    }

    /// Imports raw key bytes directly -- used by `import_master` once it has
    /// read them from the backup directory. Takes `mint_lock` (M05A A7
    /// review finding 2), the same lock `get_or_mint` holds across its own
    /// read-then-write: without it, a concurrent `adopt` can see an empty
    /// slot, mint, and store, and then have this import's write land
    /// after -- the row would then name a DID the vault no longer holds.
    /// Warns, rather than refuses, when this replaces an existing,
    /// different key (finding 1): D-A7-11's standing rule is that a
    /// master is never forgotten, and the handover A7 makes routine
    /// (`import-master` before every `adopt`) can otherwise silently
    /// destroy a live identity on a typo'd instance id or a stale backup
    /// file, with nothing to notice. Not a refusal -- a deliberate
    /// replacement (the actual handover case) must still succeed.
    pub async fn import(&self, name: &str, key_bytes: &[u8; 32]) -> Result<(), VaultError> {
        let _guard = self.mint_lock.lock().await;
        if let Some(existing) = self.get(name).await? {
            let existing_did = substrate::derive_did_key(&existing.public_key());
            let incoming_did =
                substrate::derive_did_key(&Identity::from_bytes(key_bytes).public_key());
            if existing_did != incoming_did {
                tracing::warn!(
                    name,
                    existing_did,
                    incoming_did,
                    "import replaces an existing, different key under this name in the vault -- \
                     the old identity is now unrecoverable unless it was exported first"
                );
            }
        }
        self.store_bytes(name, key_bytes).await
    }

    /// Writes `name`'s master out of the vault into the configured backup
    /// directory, returning only the path written -- no key bytes ever
    /// leave this function except into that file, through
    /// `Identity::save_to_path` (mode `0o600`).
    pub async fn export_master(&self, name: &str) -> Result<PathBuf, VaultError> {
        validate_backup_name(name)?;
        let identity = self.get(name).await?.ok_or_else(|| {
            VaultError::Storage(anyhow::anyhow!("no master named '{name}' in this vault"))
        })?;
        ensure_backup_dir(&self.backup_dir)?;
        let path = self.backup_dir.join(format!("{name}.key"));
        identity.save_to_path(&path).map_err(VaultError::Storage)?;
        Ok(path)
    }

    /// Adopts a master already placed in the backup directory (an A0-A4
    /// deployment's `identities/member-*.key`, copied there by the
    /// operator) into the vault. Reads by name from that directory only.
    /// Routes through `import` so a replacement is locked and warned
    /// about the same way a direct `import` call is.
    pub async fn import_master(&self, name: &str) -> Result<(), VaultError> {
        validate_backup_name(name)?;
        let path = self.backup_dir.join(format!("{name}.key"));
        let identity = Identity::load_from_path(&path).map_err(VaultError::Storage)?;
        self.import(name, &identity.to_bytes()).await
    }
}

/// `member-<app_instance_id>#<service_name>-<index>` -- the same computable
/// name A0's `member_master_name` uses, so an operator adopting an
/// existing deployment's masters finds the same file stem under
/// `import_master`.
///
/// The `app_instance_id`/`service_name` boundary is `#`, not `-` (M05A A5e,
/// D-A5e-12): both id types forbid `/` and `#` at construction, but neither
/// forbids `-`, so with a `-` boundary instance `a` + service `b-c` and
/// instance `a-b` + service `c` collided on the identical name -- one vault
/// master handed to two different app instances. `#` is forbidden in both,
/// so it always marks the true boundary regardless of what either segment
/// contains; the trailing `-index` boundary needs no guard, since `index`
/// is always a plain `u32` and can never itself contain a `-`, which makes
/// the rightmost `-` in the whole string unambiguous already. Kept in sync
/// with `apps/roymctl/src/commands/member_identity.rs`'s own
/// `member_master_name`.
///
/// Validated against the same rule `export_master`/`import_master` enforce
/// on the name they are handed, so a name they will later refuse is
/// refused here instead, at mint time, rather than minting a key that is
/// permanently un-backup-able and only discovered when the operator tries
/// (S5, Slice A5b review).
fn member_master_name(
    app_instance_id: &str,
    service_name: &str,
    index: u32,
) -> Result<String, VaultError> {
    let name = format!("member-{app_instance_id}#{service_name}-{index}");
    validate_backup_name(&name)?;
    Ok(name)
}

/// `app-<app_instance_id>` -- the app instance's own master (M05A A7,
/// D-A7-3). Collision-safe by construction on two independent grounds,
/// neither of which depends on what `AppInstanceId`'s validator permits
/// (unlike `member_master_name`'s `#` boundary above):
///
/// - **Injective by construction.** There is exactly one variable-length
///   segment, and it is the whole remainder of the string, so the map from
///   instance id to name has no room to collide two different ids onto one
///   name.
/// - **Disjoint from every member name.** `app-` and `member-` are fixed,
///   disjoint prefixes at position 0, so no member name can ever equal an app
///   name whatever either id's segments contain.
///
/// Still validated through `validate_backup_name`, the same rule
/// `export_master`/`import_master` enforce on the name they are handed:
/// `AppInstanceId::try_new` refuses only empty, `/`, and `#`, which is
/// narrower than what a backup name must avoid (also `\`, `..`, and an
/// absolute path), so an instance id valid at construction can still be
/// unusable as a vault key -- refused here, at mint time, rather than
/// minted and only discovered unbackuppable later.
///
/// Deliberately **no** duplicated copy of this in `roymctl`, unlike
/// `member_master_name`'s copy in `apps/roymctl/src/commands/
/// member_identity.rs`: that copy exists because A0's operator-side flow
/// mints member masters into files on the operator's own machine, and
/// nothing client-side ever mints an app master -- there is nothing to
/// keep in sync.
fn app_master_name(app_instance_id: &str) -> Result<String, VaultError> {
    let name = format!("app-{app_instance_id}");
    validate_backup_name(&name)?;
    Ok(name)
}

/// The app instance's own master: resolve-or-mint under `app-<id>`,
/// returning both the DID `adopt` records on the instance row and the
/// vault name `export-master`/`import-master` accept (M05A A7,
/// D-A7-1/D-A7-3/D-A7-4/D-A7-8). Called on *every* successful `adopt`, not
/// only the first -- resolving rather than minting-once is what makes a
/// handover through `import-master` self-correcting: an `adopt` run after
/// `import-master` resolves the imported key instead of minting a second
/// identity for the same instance (D-A7-5).
pub async fn app_master(
    vault: &MasterVault,
    app_instance_id: &str,
) -> Result<(String, String), VaultError> {
    let name = app_master_name(app_instance_id)?;
    let master = vault.get_or_mint(&name, MasterKind::App).await?;
    let did = substrate::derive_did_key(&master.public_key());
    Ok((did, name))
}

/// The member master a *renewal* signs with: read-only, under the same
/// computable name `mint_and_substitute` stored it as. Deliberately not
/// `get_or_mint` -- a placed member whose master is missing from the vault
/// is a custody failure, and minting a fresh one there would silently give
/// the member a new identity rather than renewing its old one.
///
/// `index` is the member's own ordinal (M05A A5e, D-A5e-5) -- reading it
/// from the caller rather than hardcoding `0` is what keeps member N's
/// anchor refresh and renewal from silently operating on member 0's master.
pub async fn master_for_member(
    vault: &MasterVault,
    app_instance_id: &str,
    service_name: &str,
    index: u32,
) -> Result<Identity, VaultError> {
    let name = member_master_name(app_instance_id, service_name, index)?;
    vault.get(&name).await?.ok_or_else(|| {
        VaultError::Storage(anyhow::anyhow!(
            "no member master named '{name}' in this supervisor's vault; it was never minted \
             here, or was minted and then lost -- import it with `roymctl supervisor \
             import-master {name}`"
        ))
    })
}

/// The `submit` path's own custody step (§0.30/D-A5-26): resolve-or-mint one
/// master per placed member, then substitute the compiler's fabricated
/// `service_id`s (and every `resolved_dependencies` entry naming one) for
/// the resolved master DIDs -- `member_identity::substitute_and_certify_
/// members`'s substitution half, moved here where the masters actually
/// live. Certification stays out: a certificate must be minted by whoever
/// deploys, which is the supervisor's own apply path, not this store step.
pub async fn mint_and_substitute(
    plan: &mut DeploymentPlan,
    vault: &MasterVault,
) -> Result<(Vec<MintedMaster>, BTreeMap<ServiceId, Identity>), VaultError> {
    let mut substitution: BTreeMap<ServiceId, ServiceId> = BTreeMap::new();
    let mut masters: BTreeMap<ServiceId, Identity> = BTreeMap::new();
    let mut minted = Vec::new();
    for svc in &plan.services {
        let name = member_master_name(
            &plan.app_instance_id,
            svc.logical_ref.service_name.as_str(),
            svc.member_index,
        )?;
        let master = vault.get_or_mint(&name, MasterKind::Member).await?;
        let master_did = substrate::derive_did_key(&master.public_key());
        let master_id = ServiceId::try_new(master_did.clone()).map_err(VaultError::Storage)?;
        substitution.insert(svc.service_id.clone(), master_id.clone());
        minted.push(MintedMaster {
            service_name: svc.logical_ref.service_name.to_string(),
            master_did,
            vault_name: name,
            member_index: svc.member_index,
        });
        masters.insert(master_id, master);
    }

    for svc in &mut plan.services {
        let old_id = svc.service_id.clone();
        svc.service_id = substitution.get(&old_id).cloned().ok_or_else(|| {
            VaultError::Storage(anyhow::anyhow!("no resolved member master for service {old_id}"))
        })?;
        svc.resolved_dependencies = svc
            .resolved_dependencies
            .iter()
            .map(|(name, members)| {
                let substituted = members
                    .iter()
                    .map(|dep| {
                        substitution.get(dep).cloned().ok_or_else(|| {
                            VaultError::Storage(anyhow::anyhow!(
                                "no resolved member master for dependency {dep}"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, VaultError>>()?;
                Ok((name.clone(), substituted))
            })
            .collect::<Result<BTreeMap<_, _>, VaultError>>()?;
    }

    Ok((minted, masters))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use syneroym_app_orchestration::models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, LogicalServiceRef, PlannedService,
        RotationPolicy, ServiceConfig, ServiceType, TopologyMode,
    };
    use syneroym_data_db::SqliteStorageProvider;
    use tokio::runtime::{Builder, Runtime};
    use tracing_subscriber::prelude::*;

    use super::*;

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl std::io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Runs `f` under a scoped tracing subscriber and returns its result
    /// alongside everything logged during the call, as text -- the same
    /// `MockWriter`-over-`fmt::layer` shape already used elsewhere in this
    /// tree (e.g. `crates/data_db/src/sqlite.rs`'s
    /// `test_insecure_mode_warning`), adapted to an async callee by
    /// driving a private, current-thread runtime inside the subscriber's
    /// own scope: `with_default` sets a thread-local for the duration of
    /// a *synchronous* closure, so the whole future must be polled to
    /// completion on this one thread, not handed to the outer
    /// `#[tokio::test]` runtime's own scheduler, or the capture would miss
    /// anything logged after a hop to a different worker thread.
    ///
    /// Takes `rt` by reference rather than building one per call: a
    /// `MasterVault`'s `open_service_db` spawns its writer loop with
    /// `task::spawn_blocking`, which outlives the individual `get_or_mint`/
    /// `import` call and is only joined when the *runtime* that spawned it
    /// drops. A runtime built fresh inside this function and dropped at
    /// its return would then block forever trying to join a writer thread
    /// still kept alive by a vault the *caller* goes on using -- callers
    /// build one `rt` for the whole test and declare it before the vault,
    /// so the vault (and its writer channel) drops first, at the test's
    /// own end, letting the runtime's final join succeed.
    fn run_capturing_logs<F, Fut, T>(rt: &Runtime, f: F) -> (T, String)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();
        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer);
        let subscriber = tracing_subscriber::registry().with(layer);

        let result = tracing::subscriber::with_default(subscriber, || rt.block_on(f()));

        let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        (result, logs_content)
    }

    fn test_runtime() -> Runtime {
        Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn vault(dir: &Path) -> MasterVault {
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(dir.join("db"), false).unwrap());
        let key_store = Arc::new(KeyStore::new());
        MasterVault::new(storage_provider, key_store, "supervisor".to_string(), dir.join("backups"))
    }

    #[tokio::test]
    async fn a_minted_master_round_trips_through_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let minted = v.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await.unwrap();
        let read_back = v.get("member-inst-1-backend-0").await.unwrap().unwrap();
        assert_eq!(minted.public_key(), read_back.public_key());
    }

    /// S3 (Slice A5b review): before `mint_lock`, two concurrent calls for
    /// the same name both saw `get` return `None` and both minted --
    /// whichever `write_secret` landed last silently orphaned the other's
    /// master. Runs the two calls truly concurrently (both `tokio::spawn`ed
    /// before either is awaited) rather than sequentially, so a regression
    /// that drops the lock has a real chance to interleave and fail this.
    #[tokio::test]
    async fn concurrent_get_or_mint_for_the_same_name_never_mints_twice() {
        let dir = tempfile::tempdir().unwrap();
        let v = Arc::new(vault(dir.path()));

        let v1 = v.clone();
        let a = tokio::spawn(async move {
            v1.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await
        });
        let v2 = v.clone();
        let b = tokio::spawn(async move {
            v2.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await
        });

        let first = a.await.unwrap().unwrap();
        let second = b.await.unwrap().unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }

    #[tokio::test]
    async fn resubmitting_reuses_the_masters_already_in_the_vault_rather_than_minting_again() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let first = v.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await.unwrap();
        let second = v.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await.unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }

    fn planned_service(name: &str, fabricated_id: &str) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(fabricated_id),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(name),
            },
            substrate: None,
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:9000".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::default(),
                fdae: None,
                health_check: None,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
        }
    }

    fn plan(services: Vec<PlannedService>) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::parse("1.0.0").unwrap(),
            services,
        }
    }

    #[tokio::test]
    async fn submit_mints_one_master_per_service_and_substitutes_the_plans_ids() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        let mut p = plan(vec![
            planned_service("frontend", "did:key:hFabricatedFrontend"),
            planned_service("backend", "did:key:hFabricatedBackend"),
        ]);

        let (minted, masters) = mint_and_substitute(&mut p, &v).await.unwrap();

        assert_eq!(minted.len(), 2);
        assert_eq!(masters.len(), 2);
        for svc in &p.services {
            assert_ne!(svc.service_id.as_str(), "did:key:hFabricatedFrontend");
            assert_ne!(svc.service_id.as_str(), "did:key:hFabricatedBackend");
            assert!(svc.service_id.as_str().starts_with("did:key:"));
            assert!(masters.contains_key(&svc.service_id));
        }
        // Re-running against the same plan resolves the *same* masters
        // rather than minting new ones.
        let mut p2 = plan(vec![
            planned_service("frontend", "did:key:hFabricatedFrontend"),
            planned_service("backend", "did:key:hFabricatedBackend"),
        ]);
        mint_and_substitute(&mut p2, &v).await.unwrap();
        assert_eq!(p.services[0].service_id, p2.services[0].service_id);
        assert_eq!(p.services[1].service_id, p2.services[1].service_id);
    }

    /// D-A5e-5/D-A5e-11: two members of one logical service must resolve
    /// two distinct masters, keyed on each member's own index -- the
    /// consequence of `mint_and_substitute` reading `svc.member_index`
    /// instead of the literal `0` it used to.
    #[tokio::test]
    async fn mint_and_substitute_mints_a_distinct_master_per_member_index() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        let mut member0 = planned_service("backend", "did:key:hFabricated0");
        let mut member1 = planned_service("backend", "did:key:hFabricated1");
        member1.member_index = 1;
        let mut p = plan(vec![member0.clone(), member1.clone()]);

        let (minted, masters) = mint_and_substitute(&mut p, &v).await.unwrap();
        assert_eq!(minted.len(), 2);
        assert_ne!(p.services[0].service_id, p.services[1].service_id);
        assert!(masters.contains_key(&p.services[0].service_id));
        assert!(masters.contains_key(&p.services[1].service_id));

        // Reusing the same indices resolves the same masters as before,
        // proving the key really is `(instance, service, index)`.
        member0.service_id = ServiceId::new("did:key:hFabricated0");
        member1.service_id = ServiceId::new("did:key:hFabricated1");
        let mut p2 = plan(vec![member0, member1]);
        mint_and_substitute(&mut p2, &v).await.unwrap();
        assert_eq!(p.services[0].service_id, p2.services[0].service_id);
        assert_eq!(p.services[1].service_id, p2.services[1].service_id);
    }

    /// D-A5e-5: `master_for_member` reads the caller-supplied index, not a
    /// hardcoded `0` -- member 1's renewal must sign with member 1's own
    /// master, never member 0's.
    #[tokio::test]
    async fn master_for_member_reads_the_members_own_index_rather_than_zero() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        let member0 = v.get_or_mint("member-inst-1#backend-0", MasterKind::Member).await.unwrap();
        let member1 = v.get_or_mint("member-inst-1#backend-1", MasterKind::Member).await.unwrap();
        assert_ne!(member0.public_key(), member1.public_key());

        let resolved0 = master_for_member(&v, "inst-1", "backend", 0).await.unwrap();
        let resolved1 = master_for_member(&v, "inst-1", "backend", 1).await.unwrap();
        assert_eq!(resolved0.public_key(), member0.public_key());
        assert_eq!(resolved1.public_key(), member1.public_key());
        assert_ne!(resolved0.public_key(), resolved1.public_key());
    }

    /// S5 (Slice A5b review): `AppInstanceId` used to only check non-empty,
    /// so an id containing a path separator minted fine and only failed
    /// later, at `export_master`, once the operator tried to back it up.
    /// M05A A5e's `AppInstanceId` validator now closes that one layer up --
    /// `AppInstanceId::try_new` itself refuses `/` -- but `LogicalServiceName`
    /// still permits `..` (it forbids only `/` and `#`), so the same
    /// defense-in-depth still matters one field over: a service name of
    /// `..` must still be refused at mint time, before anything is stored
    /// under it, not discovered later at `export_master`.
    #[tokio::test]
    async fn a_plan_whose_service_name_contains_a_path_traversal_is_refused_before_minting() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        let mut p = plan(vec![planned_service("..", "did:key:hFabricatedFrontend")]);
        p.app_instance_id = AppInstanceId::new("inst-1");

        let err = mint_and_substitute(&mut p, &v).await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)), "{err:?}");
        assert!(
            v.get("member-inst-1#..-0").await.unwrap().is_none(),
            "the rejected name must never have been written to the vault"
        );
    }

    /// `AppInstanceId` forbids `/` (a `validate_backup_name` concern) and
    /// `#` (`MemberRef`'s own index separator, D-A5e-2), but not `-` -- a
    /// hyphen in an instance id is ordinary and must keep working.
    #[test]
    fn an_app_instance_id_containing_a_path_separator_is_refused_at_construction() {
        assert!(AppInstanceId::try_new("a-b").is_ok());
        assert!(AppInstanceId::try_new("a/b").is_err());
    }

    /// M05A A5e (D-A5e-12): the collision `member_master_name`'s `#`
    /// boundary closes -- instance `a` + service `b-c` and instance `a-b` +
    /// service `c` used to both mint the identical vault key
    /// `member-a-b-c-0` under the old `-`-only boundary, handing one master
    /// DID to two different (instance, service) pairs. Neither id type can
    /// contain `#`, so the boundary is unambiguous regardless of what
    /// hyphens either segment carries.
    #[test]
    fn member_master_name_cannot_collide_across_two_instance_and_service_pairs() {
        let a_bc = member_master_name("a", "b-c", 0).unwrap();
        let ab_c = member_master_name("a-b", "c", 0).unwrap();
        assert_ne!(a_bc, ab_c);
    }

    #[tokio::test]
    async fn export_master_writes_only_into_the_configured_backup_dir() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        v.get_or_mint("member-inst-1-backend-0", MasterKind::Member).await.unwrap();

        let err = v.export_master("../escape").await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)));
        let err = v.export_master("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)));

        let path = v.export_master("member-inst-1-backend-0").await.unwrap();
        assert_eq!(path.parent().unwrap(), dir.path().join("backups"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(file_mode, 0o600);
            let dir_mode =
                fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700);
        }
    }

    #[tokio::test]
    async fn import_master_reads_only_from_the_configured_backup_dir() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let err = v.import_master("../escape").await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)));
        let err = v.import_master("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)));

        // A name that resolves inside the backup dir but names no file
        // there fails as a storage error (file not found), not as a path
        // violation -- confirms the read really is scoped to that
        // directory rather than silently succeeding elsewhere.
        let err = v.import_master("never-placed").await.unwrap_err();
        assert!(matches!(err, VaultError::Storage(_)));
    }

    #[tokio::test]
    async fn a_locked_vault_is_reported_as_vault_locked_not_as_a_storage_error() {
        let dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(dir.path().join("db"), true).unwrap());
        let key_store = Arc::new(KeyStore::new());
        let v = MasterVault::new(
            storage_provider,
            key_store,
            "supervisor".to_string(),
            dir.path().join("backups"),
        );

        let err = v.get("member-inst-1-backend-0").await.unwrap_err();
        assert!(matches!(err, VaultError::Locked), "{err:?}");
    }

    // ── M05A A7: the app-instance master (D-A7-3/D-A7-5) ────────────────

    /// D-A7-3: the two prefixes are disjoint at position 0, so no member
    /// name can ever equal an app name -- checked over instance ids chosen
    /// to look like a member name's own tail, the shape most likely to
    /// collide under a careless boundary. The companion to
    /// `member_master_name_cannot_collide_across_two_instance_and_service_pairs`
    /// above, which pins the *member* name's own boundary.
    #[test]
    fn an_app_master_name_can_never_equal_a_member_master_name() {
        for instance_id in ["inst-1", "backend-0", "member-inst-1#backend-0"] {
            let app_name = app_master_name(instance_id).unwrap();
            let member_name = member_master_name(instance_id, "svc", 0).unwrap();
            assert_ne!(app_name, member_name);
            assert!(app_name.starts_with("app-"));
            assert!(member_name.starts_with("member-"));
        }
    }

    /// D-A7-3, §0.3 as corrected in review: `AppInstanceId` permits `..`
    /// and `\` (it forbids only empty, `/`, and `#`), but
    /// `validate_backup_name` refuses both -- so an instance id valid at
    /// construction must still be refused *before* anything is minted
    /// under it, not discovered later at `export_master`.
    #[tokio::test]
    async fn an_app_instance_id_that_could_not_be_backed_up_is_refused_before_minting() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        for instance_id in ["..", "back\\slash"] {
            let err = app_master(&v, instance_id).await.unwrap_err();
            assert!(matches!(err, VaultError::Storage(_)), "{err:?}");
        }
        // Neither rejected name was written to the vault under the name
        // `app_master_name` would otherwise have produced.
        assert!(v.get("app-..").await.unwrap().is_none());
        assert!(v.get("app-back\\slash").await.unwrap().is_none());
    }

    /// D-A7-5: `adopt` calls `app_master` on every call, not only the
    /// first -- resolving the same key it already minted is what makes a
    /// re-`adopt` (and, across two vaults, a handover) safe rather than
    /// minting a second identity.
    #[tokio::test]
    async fn a_second_resolve_returns_the_app_master_already_in_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let (first_did, first_name) = app_master(&v, "inst-1").await.unwrap();
        let (second_did, second_name) = app_master(&v, "inst-1").await.unwrap();
        assert_eq!(first_did, second_did);
        assert_eq!(first_name, second_name);
        assert_eq!(first_name, "app-inst-1");
    }

    /// `task.md`'s own exit criterion: the app master "moves through
    /// `export-master` / `import-master`" exactly as a member master does
    /// -- exported under `app-<id>`, landing inside the configured backup
    /// directory at mode `0o600`, and re-importable to restore the same
    /// key.
    #[tokio::test]
    async fn an_app_master_round_trips_through_export_master_and_import_master() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let (did, vault_name) = app_master(&v, "inst-1").await.unwrap();
        assert_eq!(vault_name, "app-inst-1");

        let path = v.export_master(&vault_name).await.unwrap();
        assert_eq!(path.parent().unwrap(), dir.path().join("backups"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Importing into a second, independent vault (a stand-in for a
        // second supervisor) restores the identical key.
        let dir2 = tempfile::tempdir().unwrap();
        let v2 = MasterVault::new(
            Arc::new(SqliteStorageProvider::new(dir2.path().join("db"), false).unwrap()),
            Arc::new(KeyStore::new()),
            "supervisor".to_string(),
            dir.path().join("backups"),
        );
        v2.import_master(&vault_name).await.unwrap();
        let imported = v2.get(&vault_name).await.unwrap().unwrap();
        assert_eq!(substrate::derive_did_key(&imported.public_key()), did);
    }

    // ── M05A A7 code review (2026-08-04): findings 1, 2, 7b, 10 ─────────

    /// D-A7-9, review finding 10: `get_or_mint`'s warning names the right
    /// noun and the right loss for each `MasterKind` -- the whole payload
    /// of a decision that otherwise touched twelve call sites for no
    /// externally visible effect.
    #[test]
    fn get_or_mint_warns_with_the_wording_matching_its_kind() {
        // Declared before `dir`/`v`: dropped last, after the vault (and
        // its background writer thread) is already gone -- see
        // `run_capturing_logs`'s own doc for why the other order deadlocks.
        let rt = test_runtime();
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let (_, member_logs) = run_capturing_logs(&rt, || {
            v.get_or_mint("member-inst-1#backend-0", MasterKind::Member)
        });
        assert!(member_logs.contains("member master"), "{member_logs}");
        assert!(member_logs.contains("orphans every row"), "{member_logs}");

        let (_, app_logs) =
            run_capturing_logs(&rt, || v.get_or_mint("app-inst-1", MasterKind::App));
        assert!(app_logs.contains("app-instance master"), "{app_logs}");
        assert!(app_logs.contains("network identity"), "{app_logs}");
    }

    /// Review finding 7b (the ordering `import` taking `mint_lock` makes
    /// safe, §0.5's own handover sequence): if `import-master` lands
    /// *before* anything has minted under this name, `get_or_mint`
    /// resolves the imported key rather than minting a second identity --
    /// the exact "`import-master` before `adopt`" order the developer
    /// guide recommends.
    #[tokio::test]
    async fn get_or_mint_after_an_import_resolves_the_imported_key_rather_than_minting() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        let imported = Identity::generate().unwrap();

        v.import("app-inst-1", &imported.to_bytes()).await.unwrap();
        let resolved = v.get_or_mint("app-inst-1", MasterKind::App).await.unwrap();

        assert_eq!(resolved.public_key(), imported.public_key());
    }

    /// Review findings 1 and 2 together: if a mint already landed under
    /// this name before `import` runs, `import` still wins (a deliberate
    /// handover replacement must succeed), but it must never do so
    /// silently -- the exact "reverse interleaving" finding 2 called
    /// worse than the base case, because nothing before this fix warned
    /// at all. Sequential, not raced: the two lock-acquisition orderings
    /// this fix distinguishes are each individually deterministic, and
    /// this is the one where `import` runs second. Not `#[tokio::test]`:
    /// `run_capturing_logs` drives its own runtime, and nesting one
    /// inside another panics.
    #[test]
    fn import_over_an_existing_different_key_replaces_it_and_warns() {
        let rt = test_runtime();
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let ((minted_did, replacement_did, final_did), logs) = run_capturing_logs(&rt, || async {
            let minted = v.get_or_mint("app-inst-1", MasterKind::App).await.unwrap();
            let minted_did = substrate::derive_did_key(&minted.public_key());

            let replacement = Identity::generate().unwrap();
            let replacement_did = substrate::derive_did_key(&replacement.public_key());
            v.import("app-inst-1", &replacement.to_bytes()).await.unwrap();

            let final_identity = v.get("app-inst-1").await.unwrap().unwrap();
            let final_did = substrate::derive_did_key(&final_identity.public_key());

            (minted_did, replacement_did, final_did)
        });

        assert_ne!(minted_did, replacement_did);
        assert_eq!(final_did, replacement_did, "import must still win over the earlier mint");
        assert!(logs.contains("replaces an existing, different key"), "{logs}");
    }

    /// The companion case: importing the *same* key a second time (an
    /// idempotent re-run of a handover step) must not warn -- there is
    /// nothing to lose, since the incoming and existing DIDs are equal.
    #[test]
    fn importing_the_same_key_again_does_not_warn() {
        let rt = test_runtime();
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let ((), logs) = run_capturing_logs(&rt, || async {
            let identity = Identity::generate().unwrap();
            let bytes = identity.to_bytes();
            v.import("app-inst-1", &bytes).await.unwrap();
            v.import("app-inst-1", &bytes).await.unwrap();
        });

        assert!(!logs.contains("replaces an existing"), "{logs}");
    }
}
