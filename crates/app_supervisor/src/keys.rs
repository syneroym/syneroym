//! Member master keys, held in the supervisor's own encrypted service vault
//! rather than on disk beside a config (ADR-0020 §4). Keyed by the same
//! computable name A0 uses for files (`member-<instance>-<service>-<index>`),
//! so adoption is a read.
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
    /// master rather than minting a second one for the same name.
    pub async fn get_or_mint(&self, name: &str) -> Result<Identity, VaultError> {
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
        tracing::warn!(
            name,
            did,
            "minted a new member master into the supervisor vault -- back it up with `roymctl \
             supervisor export-master {name}`; losing it is unrecoverable and orphans every row \
             this member has written"
        );
        Ok(identity)
    }

    /// Imports raw key bytes directly -- used by `import_master` once it has
    /// read them from the backup directory.
    pub async fn import(&self, name: &str, key_bytes: &[u8; 32]) -> Result<(), VaultError> {
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
    pub async fn import_master(&self, name: &str) -> Result<(), VaultError> {
        validate_backup_name(name)?;
        let path = self.backup_dir.join(format!("{name}.key"));
        let identity = Identity::load_from_path(&path).map_err(VaultError::Storage)?;
        self.store_bytes(name, &identity.to_bytes()).await
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
        let master = vault.get_or_mint(&name).await?;
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
    use std::collections::BTreeMap;

    use syneroym_app_orchestration::models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, LogicalServiceRef, PlannedService,
        RotationPolicy, ServiceConfig, ServiceType, TopologyMode,
    };
    use syneroym_data_db::SqliteStorageProvider;

    use super::*;

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

        let minted = v.get_or_mint("member-inst-1-backend-0").await.unwrap();
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
        let a = tokio::spawn(async move { v1.get_or_mint("member-inst-1-backend-0").await });
        let v2 = v.clone();
        let b = tokio::spawn(async move { v2.get_or_mint("member-inst-1-backend-0").await });

        let first = a.await.unwrap().unwrap();
        let second = b.await.unwrap().unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }

    #[tokio::test]
    async fn resubmitting_reuses_the_masters_already_in_the_vault_rather_than_minting_again() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());

        let first = v.get_or_mint("member-inst-1-backend-0").await.unwrap();
        let second = v.get_or_mint("member-inst-1-backend-0").await.unwrap();
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
        let member0 = v.get_or_mint("member-inst-1#backend-0").await.unwrap();
        let member1 = v.get_or_mint("member-inst-1#backend-1").await.unwrap();
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
        v.get_or_mint("member-inst-1-backend-0").await.unwrap();

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
}
