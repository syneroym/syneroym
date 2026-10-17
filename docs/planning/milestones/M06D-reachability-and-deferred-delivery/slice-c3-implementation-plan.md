# M06D slice C3 — Record, keys and lookup: implementation plan

*Reader: the developer (or AI) who implements slice C3. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "C3", "C4",
  "C8", "K-2", "D-C3-n" in code, comments or test names; ADR ids are fine),
  simple English in comments, the 800-line file cap, the 100-line function
  cap, the module layout rule (`foo.rs` beside `foo/`, never `mod.rs` under
  `src/`), and no hard-coded ports in substrate tests.
  **This plan names its decisions `C3-1` to `C3-14`. Those ids are for this
  plan only. When a step says to write a comment "for the reason in C3-n",
  write the reason itself in the comment, never the id.**
- `task.md` in this folder: §1 (shared parts), §2 items 5 and 6, §3 slice C3
  and the slices that use it (C4, C5, C6, C7, C8).
- `docs/planning/changes/service-mailbox/change.md`: criteria K-1 to K-8,
  M-3, R-1, S-1; Design sections "Service record and its life", "Mailbox
  key", "Instance certificate lifetime"; `## Open questions` (first item);
  `## Deviations` (`D-C2-3`, `D-C2-6`).
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) Decisions 1
  and 4; [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
  §1, §3, §4 and §6 (member master, instance certificate, records signed by
  the master and replayed verbatim by the substrate);
  [ADR-0018](../../../decisions/0018-service-record-visibility.md) §4
  (public, internal, private).
- `slice-c2-implementation-plan.md` §5 (what C3 takes from C2).
- The code this slice builds on (read first):
  - `crates/mailbox_format/src/keys.rs`: `MailboxSecretKey`,
    `MailboxPublicKey` (z-base-32 text, serde as a string).
  - `crates/identity/src/keys.rs:242-259`: `Identity::derive_service_identity`
    (the HKDF pattern C3-1 copies), `Identity::to_bytes` (:185).
  - `crates/core/src/dht_registry/types.rs`: `EndpointInfo`,
    `SignedEndpointInfo::verify` (it compares the whole parsed struct).
  - `crates/core/src/dht_registry/client.rs:185-270`: `RegistryClient::lookup`.
  - `crates/router/src/net_iroh.rs:150-171`: `resolve_iroh_addr`.
  - `crates/router/src/proxy/router.rs:296-305`: `invoke_remote`.
  - `crates/control_plane/src/service/orchestration/deploy.rs` and
    `deploy/{admission,commit,manifest,provision}.rs`, `cert.rs`,
    `lifecycle.rs:249-457` (undeploy), `types.rs:24-120`.
  - `crates/sdk/src/deploy/{certify.rs,types.rs}`, `crates/sdk/src/deploy.rs`,
    `crates/sdk/src/mapper.rs:381-460`.
  - `apps/roymctl/src/commands/svc.rs:47-260`, `svc/deploy.rs`,
    `app.rs:131-279`, `app/deploy.rs`, `member_identity.rs`.
  - `crates/app_supervisor/src/service/{bindings.rs,renewal.rs}`.
  - `crates/app_orchestration/src/models/{service.rs,plan.rs}`,
    `compiler.rs`.

**Branch.** C3 needs C2 (`crates/mailbox_format`). Today C2 is on
`feat/m06d-c2-implementation` (pushed) and local `main` is still at
`cec9f26b` (B3). Cut `feat/m06d-c3-implementation` from `main` **after** the
C2 PR has merged (the serial order in task.md §4: one slice per PR, each PR
merges before the next starts). If the owner wants to start earlier, cut it
from `feat/m06d-c2-implementation` and rebase after the merge.

## 1. What C3 builds, in one paragraph

A service member can now name a **mailbox** (ADR-0026). The deployer
(`roymctl svc deploy --mailbox`, `roymctl app deploy` with a per-service
manifest field, or the supervisor) puts the mailbox's service id and the
member's mailbox **public** key into the signed service record
(`EndpointInfo.mailbox`, K-1). The mailbox key pair is **derived from the
member master key** (C3-1), so every holder of the master (roymctl, the
supervisor) gets the same key on every deploy, on any substrate, with no new
key storage (K-2). The **private** half travels to the substrate in a new
`deploy-manifest` field and is kept in the service's encrypted vault under a
reserved name that guests cannot read (C3-2, C3-3). A per-service setting
gives the instance certificate a longer lifetime (for a phone) (K-8). On the
sender side, a new `resolve_peer` returns the record's mailbox field even
when the receiver's substrate record has expired (K-5), and a small durable
**peer record store** keeps the last signed record of each peer that has a
mailbox, used when the registry cannot be reached (K-6). Nothing deposits,
pulls or runs a mailbox yet: C4 to C6 do. The behaviour of every existing
call stays the same.

## 2. Decisions this plan makes (check them before you start)

Each fills a gap in task.md or the change doc (the gaps are listed in §6).
None of them needs the owner, except where §6 says "owner check".

| # | Decision | Why |
| --- | --- | --- |
| C3-1 | **The mailbox secret is derived from the member master key**, not generated at random: `seed = HKDF-SHA256(salt = none, ikm = master.to_bytes(), info = b"syneroym:mailbox-key:v1")`, 32 bytes, then `MailboxSecretKey::from_seed(seed)`. New function `MailboxSecretKey::derive_from_master(master: &Identity) -> Self` in `crates/mailbox_format/src/keys.rs`. `MailboxSecretKey::generate` stays (tests use it). | K-2 needs the same key at every later deploy of a member. The master is already stored and backed up by both deployers (roymctl `identities/<name>.key`, supervisor `MasterVault`), so derivation needs **no new key file, no new vault row on the deployer side, and no new backup duty**, and members minted before C3 get a key too. "Made with the member master" (K-2) holds literally. HKDF is one-way, so a substrate that holds the mailbox secret learns nothing about the master. It is the same pattern as `derive_service_identity`. Cost: rotating the mailbox key needs a new `info` version (`v2`) and a new record; rotation is already deferred (backlog "Service mailbox: later improvements"). Record as a deviation (`D-C3-1`). |
| C3-2 | **The deploy-call field.** WIT `deploy-manifest` gets `mailbox: option<mailbox-credentials>`, with `record mailbox-credentials { secret-key: string }` (z-base-32 seed, the `MailboxSecretKey::to_z32` form). C8 adds `child-token: option<string>` **to this record**. It is not added now. | A record, not a bare string, is the "room for the owner's child token" task.md asks for: C8 then changes only the few places that build `mailbox-credentials` (§3 Step 6 lists them), not the 82 places that build a `DeployManifest`. An unused field now would be dead code. It is in `deploy-manifest`, so `deploy` and `deploy-plan` both carry it. `renew-cert` does not: the key never changes. |
| C3-3 | **Where the instance keeps the secret:** the service's own vault (`ServiceStore::write_secret`, AES-256-GCM under the service DEK, in the SQLCipher `state.db`), under the fixed name `syneroym.mailbox-secret`. Every name that starts with `syneroym.` is **reserved for the substrate**: the guest `vault.reveal` (WASM host and native host) and the operator RPC `security/set-secret` refuse it. New trait method `ServiceStore::delete_secret`. | It is the only encrypted per-service secret store (`endpoints.db`, which holds the instance certificate, is plaintext; §6 item 4). Today a guest can `reveal` **any** vault name, so without the reserved prefix a guest could read its own mailbox secret and a guest bug could leak it. The prefix also stops an operator from overwriting the key by hand. The change doc's "*Unverified:* ... whether the vault interface is the right place" is answered: the vault **storage** yes, the guest vault **interface** no. |
| C3-4 | `EndpointInfo` gets `#[serde(default, skip_serializing_if = "Option::is_none")] pub mailbox: Option<EndpointMailbox>`, with `pub struct EndpointMailbox { pub service_id: String, pub key: MailboxPublicKey }` in `crates/core/src/dht_registry/types.rs`. `syneroym-core` gains a dependency on `syneroym-mailbox-format`. | `skip_serializing_if` keeps every record without a mailbox byte-identical. A typed key makes a malformed key a parse error at the signer, not at C5. `core` is not compiled into any guest (only `roym_web` uses it, as a dev-dependency), and `mailbox_format` depends only on `identity`, so there is no cycle and no guest cost. |
| C3-5 | **Only a `public` member may have a mailbox.** `private` and `internal` are refused at four places: the manifest compiler, `roymctl svc deploy`, `roymctl app deploy`, and the substrate's deploy admission. Confirmed by the owner on 2026-10-17; mailboxes for private and internal services are a backlog item (§5 item 6). | K-7 says a private service has no mailbox field. An `internal` record is marked `is_private` (ADR-0018 §4) and must stay in the local registry, but the mailbox republishes the record (K-4, C4) to **its own** registry. That would leak it. So `internal` is refused too (the change doc says so in "Private and internal services"). See §6 item 3 for the `--record-out` sentence. |
| C3-6 | **The substrate checks the pair at deploy.** With `manifest.mailbox = Some(c)`: `c.secret_key` parses; visibility is `public`; a registry certificate is present and its `info.mailbox` is `Some(m)` with `m.key == secret.public_key()`; an instance certificate is present. With `info.mailbox = Some` but `manifest.mailbox = None`: refused. | Each refused case is a deploy that cannot work: senders would seal to a key the instance cannot open, or the receiver could not `register` (M-3 needs the instance certificate). Failing at deploy is cheaper than a silent loss later. |
| C3-7 | The deploy idempotency hash needs **no new input**: `stable_registry_certificate_for_hash` adds `&signed.info.mailbox`, and C3-6 makes the record's key equal to the secret's public key. The secret itself never enters the hash input. | A redeploy that changes the mailbox changes the record, so it is not skipped as a no-op. Hashing the secret would put a function of it into `service_deploy_facts.manifest_hash` for no gain. |
| C3-8 | **Per-service manifest settings, outside `ServiceConfig`:** `ServiceSpec.mailbox: Option<ServiceId>` and `ServiceSpec.instance_cert_lifetime_hours: Option<u32>`, copied by the compiler to the same two fields on `PlannedService`. Both `#[serde(default, skip_serializing_if = "Option::is_none")]`. | Same reason as `schedule` (`service.rs:327-334`): `ServiceConfig` maps onto the substrate deploy manifest and its dedup hash, and the substrate needs neither value (the mailbox reaches it inside the signed record). They must be on the **plan** because the supervisor holds no manifest. Per service, not per member index: the manifest has no per-member level (§6 item 5). |
| C3-9 | **Certificate lifetime (K-8).** Allowed range 1 to 720 hours (720 = the substrate's `MAX_INSTANCE_CERT_LIFETIME_SECS`, 30 days, `cert.rs:10`). roymctl `svc deploy --cert-lifetime-hours <N>` (default 24, the current value). `app deploy` and the supervisor use `svc.instance_cert_lifetime_hours`, else their current default (roymctl 24; supervisor `renewed_cert_expires_hours`, default 4). The supervisor renewal uses the same per-service value. | One lifetime source per member, read at first mint and at every renewal, so the two cannot disagree. The renewal moment already scales with the lifetime (last 25 %, `delegation.rs:36-46`). |
| C3-10 | `certify_placed_members` returns a struct `CertifiedMembers { instance_certificates, registry_certificates, mailbox_keys }`; `ApplyRequest` and `map_deployment_plan_to_wit` take `members: &CertifiedMembers` in place of the two certificate maps. | One value instead of three parallel maps on every call. Each `ApplyRequest` literal changes two lines into one. `CertifiedMembers` derives `Default` for the "no masters" path. |
| C3-11 | **`resolve_peer` is new; `resolve_iroh_addr` keeps its signature** and becomes a thin wrapper. `resolve_peer` reads the service record **without** `resolve = true`, keeps its `mailbox`, then looks up the substrate record. A failed substrate lookup is an error only when the record has no mailbox. | K-5. The relay path (`route_handler/io.rs:484`) and the four tests never need the mailbox, so they stay unchanged. `RegistryClient::lookup` is not changed. |
| C3-12 | **Peer record store (K-6):** `crates/router/src/peer_records.rs`, one SQLite file `<storage.db_dir>/peer_records.db`, plaintext, one row per peer. It keeps **only service records that have a mailbox**. A newer record of that peer **without** a mailbox deletes the row. Rows past `not_after` are deleted when read and at open. A newer pkarr timestamp replaces an older one, never the reverse. | A stored record helps only to deposit; without a mailbox field it has no use, so storing every peer would grow the file for nothing. The timestamp rule is the registry's own rule (ADR-0020 §6), so a stale copy never wins. Plaintext like `endpoints.db`: records are public data. The contact list it reveals is noted in the backlog (§5). |
| C3-13 | `ProxyRouter` gets the store through a new builder `with_peer_records(Arc<PeerRecordStore>)` (like `with_outbox`), built in `RouteHandler::build_proxy` from `config.storage.db_dir`. A failure to open it logs a warning and leaves it `None`. | No constructor change (16+ test sites). The store helps only offline peers; it must not stop a node from starting. |
| C3-14 | `roymctl svc deploy --mailbox auto` and `app deploy --mailbox <name>=auto` fail with "`auto` is not supported yet; give the mailbox service id". Any other value must be a `did:key:` id. | `auto` needs the registry list (C7). Parsing the word now gives a clear message instead of "not a DID". |

## 3. Step-by-step changes

Do the steps in order. Each step names every file and call site. Counts were
taken on `feat/m06d-c2-implementation` at `8d8533b2`; re-run the given
`grep` before you edit, in case `main` moved.

### Step 1 — `MailboxSecretKey::derive_from_master` (`crates/mailbox_format`)

1. `crates/mailbox_format/Cargo.toml`: add `hkdf.workspace = true` (workspace
   `hkdf = "0.12"`, root `Cargo.toml:175`, already used by `identity`).
2. `crates/mailbox_format/src/keys.rs`, in `impl MailboxSecretKey`:

```rust
/// The HKDF `info` for the mailbox key of a member master. A new version
/// string gives a new, unrelated key.
const MASTER_DERIVATION_INFO: &[u8] = b"syneroym:mailbox-key:v1";

/// The mailbox key of the member whose master is `master`. Every holder of
/// the master gets the same key, so a member keeps one mailbox key on every
/// deploy and every substrate. HKDF is one-way: the key tells nothing about
/// the master.
#[must_use]
pub fn derive_from_master(master: &Identity) -> Self {
    let ikm = Zeroizing::new(master.to_bytes());
    let mut seed = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(None, &*ikm)
        .expand(MASTER_DERIVATION_INFO, &mut *seed)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    Self::from_seed(*seed)
}
```

   `expect` is allowed only if the crate's lints permit it in non-test code;
   if clippy denies `expect_used`, map the impossible error the way
   `crates/identity/src/keys.rs:242-259` does and copy its pattern exactly.
   Imports: `use hkdf::Hkdf; use sha2::Sha256; use syneroym_identity::Identity;`.
3. Tests in `crates/mailbox_format/src/keys/tests.rs` (append):
   - `the_same_master_gives_the_same_mailbox_key` (two calls, equal
     `public_key()`).
   - `different_masters_give_different_mailbox_keys`.
   - `the_derived_key_has_a_fixed_value` — a fixed-seed master
     (`Identity::from_bytes(&[7u8; 32])`) gives a public key equal to a
     z-base-32 constant written in the test. Compute it once, paste it,
     and never change it: it pins the derivation for every later version.
   - `a_derived_key_opens_what_was_sealed_to_it` — seal with the existing
     test helpers to `derive_from_master(m).public_key()`, open with
     `derive_from_master(m)`.

### Step 2 — `EndpointInfo.mailbox` (`crates/core`)

1. `crates/core/Cargo.toml`: add `syneroym-mailbox-format.workspace = true`.
   The root `Cargo.toml` `[workspace.dependencies]` has no
   `syneroym-mailbox-format` entry yet (C2 had no user). Add
   `syneroym-mailbox-format = { path = "crates/mailbox_format" }` next to
   `syneroym-identity` (root `Cargo.toml:60`).
2. `crates/core/src/dht_registry/types.rs`:

```rust
/// The mailbox a service member chose at deploy (ADR-0026): where a sender
/// deposits a message when the member cannot be reached, and the key the
/// message is sealed to. Signed with the rest of the record, so only the
/// holder of the member master can set or change it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointMailbox {
    /// The mailbox service. A sender finds it with an ordinary registry
    /// lookup.
    pub service_id: String,
    /// The member's mailbox public key.
    pub key: MailboxPublicKey,
}
```

   and in `EndpointInfo`, after `generation`:

```rust
    /// The member's mailbox, if it chose one. Absent on every substrate
    /// record and on every service without a mailbox; omitted from the
    /// signed JSON when absent, so those records are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox: Option<EndpointMailbox>,
```

   Re-export `EndpointMailbox` wherever `EndpointInfo` is re-exported
   (`crates/core/src/dht_registry.rs`; check with
   `grep -n 'EndpointInfo' crates/core/src/dht_registry.rs crates/core/src/lib.rs`).
3. **Every `EndpointInfo { .. }` literal gets `mailbox: None`** (53 sites, no
   site uses `..base`). Find them with
   `grep -rn 'EndpointInfo {' --include='*.rs' crates apps tests | grep -v 'pub struct'`.
   Non-test (12): `crates/sdk/src/deploy/certify.rs:160` (changed for real
   in Step 7), `apps/roymctl/src/commands/svc/deploy.rs:443` (changed for
   real in Step 8), `apps/roymctl/src/commands/registry.rs:69`,
   `crates/substrate/src/runtime/publish.rs:236`,
   `crates/app_supervisor/src/tier1.rs:179`,
   `crates/coordinator_iroh/src/registration.rs:139`,
   `crates/smoke-tests/src/main.rs:187`,
   `tests/perf/src/scenarios/{wasm_latency.rs:205,tcp_proxy_latency.rs:81,concurrency.rs:78,soak.rs:115,soak.rs:322}`.
   Test (41): `crates/core/src/endpoint_publisher.rs:207`,
   `crates/core/src/dht_registry/tests.rs:8`,
   `crates/coordinator_webrtc/src/bootstrap.rs` (6),
   `crates/substrate/tests/*.rs` (19 across 15 files, plus
   `common/conversation_fixture.rs:43`), `crates/sdk/src/tests.rs` (2),
   `crates/sdk/src/topology/tests.rs:136`,
   `crates/control_plane/src/service/orchestration/tests/helpers.rs:1004`,
   `crates/community_registry/src/registry/tests.rs` (8),
   `crates/coordinator_iroh/tests/multi_hop_relay.rs` (2).
   `tests/perf` is outside the workspace build graph only if the root
   `Cargo.toml` excludes it; check, and update it either way so it still
   builds.
4. `crates/control_plane/src/service/orchestration/types.rs:38-55`
   (`stable_registry_certificate_for_hash`): add `&signed.info.mailbox` as
   the last tuple element, and add one sentence to its doc: the mailbox is
   part of what the service is (C3-7 reason).
5. Tests in `crates/core/src/dht_registry/tests.rs`:
   - `a_record_with_a_mailbox_signs_and_verifies` (round trip,
     `verify()` ok, parsed `mailbox` equal).
   - `a_record_without_a_mailbox_has_no_mailbox_key_in_its_json`
     (`!json.contains("mailbox")`).
   - `a_service_record_with_a_mailbox_fits_the_dht_size_limit`: a service
     record with a 64-character nickname, a `did:key` `service_id` and
     `substrate_id`, and a mailbox; `sign` succeeds (pkarr refuses a packet
     over 1000 bytes) and `signed_packet.encoded_packet().len() <= 1000`
     (use the pkarr accessor that exists; check its name in the cached
     crate source, not docs).
   - `a_record_with_a_malformed_mailbox_key_does_not_parse`.

### Step 3 — WIT field (`crates/wit_interfaces`)

`crates/wit_interfaces/wit/control-plane/control-plane.wit`, before
`record deploy-manifest`:

```wit
    /// What an instance needs to use its member's mailbox (ADR-0026).
    /// The substrate keeps it in the service's vault, where the guest
    /// cannot read it. Never log a manifest that carries this record.
    record mailbox-credentials {
        /// The member's mailbox secret key: z-base-32 of its 32-byte seed.
        secret-key: string,
    }
```

and in `record deploy-manifest`, after `instance-certificate`:

```wit
        /// Present exactly when the registry certificate names a mailbox.
        /// Requires visibility `public` and an instance certificate.
        mailbox: option<mailbox-credentials>,
```

Rebuild (`cargo build -p syneroym-wit-interfaces`). The generated Rust type
is `MailboxCredentials { secret_key: String }` and `DeployManifest` gains
`mailbox: Option<MailboxCredentials>`. Read
`crates/wit_interfaces/src/control_plane.rs`: if it re-exports generated
records by name, re-export `MailboxCredentials` the same way.

**Every `DeployManifest { .. }` literal gets `mailbox: None`** (82 literals
in 33 files; find with
`grep -rn 'DeployManifest {' --include='*.rs' crates apps | grep -v '\-> DeployManifest {'`).
Production: `crates/sdk/src/client/orchestrator.rs:60,102,142` (the wasm
one changes for real in Step 8), `crates/sdk/src/mapper.rs:393` (changes
for real in Step 7), `crates/sandbox_wasm/benches/{data_layer_bench.rs,abac_bench.rs}`.
Tests: `crates/control_plane/src/service/orchestration/tests/*.rs` and
`crates/control_plane/src/service/tests.rs` (52), and 20 integration-test
files listed by the grep (23). Also check the WASM test components and the
TypeScript e2e for a JSON deploy body: `grep -rn 'instance_certificate\|instance-certificate' test-components crates/substrate/tests/e2e --include='*.ts' --include='*.json' --include='*.rs'`.
A JSON caller that omits `mailbox` still parses (serde reads a missing
`Option` as `None`), so only Rust literals must change.

### Step 4 — reserved vault names and `delete_secret` (`crates/data_db`)

1. New file `crates/data_db/src/vault_names.rs`, declared `pub mod
   vault_names;` in `crates/data_db/src/lib.rs`:

```rust
//! Vault entry names the substrate keeps for itself. A guest cannot read
//! or write them through the vault interface.

/// Every name with this prefix belongs to the substrate.
pub const RESERVED_PREFIX: &str = "syneroym.";
/// The member's mailbox secret key (32-byte seed).
pub const MAILBOX_SECRET: &str = "syneroym.mailbox-secret";

#[must_use]
pub fn is_reserved(name: &str) -> bool {
    name.starts_with(RESERVED_PREFIX)
}
```

2. `crates/data_db/src/traits.rs:133-139`, trait `ServiceStore`, after
   `reveal_secret`:

```rust
    /// Removes a secret from the service's private vault. Removing a name
    /// that does not exist is not an error.
    async fn delete_secret(&self, key: &str) -> anyhow::Result<()>;
```

   Implement it in both impls in
   `crates/data_db/src/sqlite/service_store.rs` (`impl ServiceStore for
   SqliteServiceStore` at :262 and the `Arc` forwarder at :589), with
   `DELETE FROM {VAULT_TABLE} WHERE key = ?1`, in the same style as
   `write_secret` (:174-199). Confirm there is no other impl:
   `grep -rn 'impl.*ServiceStore for' crates apps`.
3. Refuse reserved names at every guest or operator door:
   - WASM guest: `crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:20`
     (`vault::Host::reveal`): first line
     `if vault_names::is_reserved(&key) { return Err(VaultError::NotFound); }`.
     `NotFound`, not a new error: a guest must not learn that the name
     exists.
   - Native guest: `crates/control_plane/src/synsvc_native/vault_config.rs:16`
     (`"reveal"` arm): the same check after `parse_params`, returning
     `Err(internal(VaultError::NotFound.to_string()))` exactly as the
     not-found branch does.
   - Operator RPC: `crates/control_plane/src/service/dispatch.rs:122`
     (`"set-secret"`): refuse with
     `RpcError::InvalidParams(format!("vault name '{key}' is reserved for the substrate"))`.
   - Search for any other door:
     `grep -rn 'reveal_secret\|write_secret' crates apps --include='*.rs' | grep -v '^crates/data_db'`.
     The supervisor's `MasterVault` (`app_supervisor/src/keys.rs`) uses its
     own service's vault with `member-`/`app-` names; it is not a guest
     door and needs no change.
4. Tests:
   - `crates/data_db/src/sqlite/tests.rs`: `test_vault_delete_secret`
     (write, delete, reveal is `None`, delete again is ok).
   - `crates/data_db/src/vault_names.rs` in-file or sibling tests:
     `is_reserved` true for `MAILBOX_SECRET`, false for `"member-x"` and
     `"syneroym"` (no dot).
   - WASM host: a guest `reveal("syneroym.mailbox-secret")` returns
     `NotFound` even when the row exists. Put it next to the existing vault
     test of `sandbox_wasm` (find it with `grep -rn 'reveal' crates/sandbox_wasm/tests`);
     if no guest fixture calls `reveal`, test at the `vault::Host` level the
     way `capabilities_store.rs` tests are written.
   - Native host and `set-secret`: one test each in the control-plane test
     module that already covers `dispatch_vault` / `set-secret` (find with
     `grep -rn 'set-secret\|"reveal"' crates/control_plane/src`).

### Step 5 — the mailbox key on the substrate (`crates/control_plane`)

1. New file `crates/control_plane/src/service/orchestration/mailbox_key.rs`
   (declare `mod mailbox_key;` in `orchestration.rs` with the other
   modules), holding the three vault operations, so C6 reads the key
   through one function:

```rust
impl ControlPlaneService {
    /// Stores the member's mailbox secret in the service's vault.
    pub(super) async fn store_mailbox_key(
        &self, service_id: &str, key: &MailboxSecretKey,
    ) -> anyhow::Result<()>;

    /// Removes it. A vault that cannot be opened is only a warning here:
    /// a key left behind is never used, because a service is pulled for
    /// only while its record names a mailbox.
    pub(super) async fn clear_mailbox_key(&self, service_id: &str);

    /// The member's mailbox secret, if this service has one.
    pub async fn mailbox_key(&self, service_id: &str)
        -> anyhow::Result<Option<MailboxSecretKey>>;
}
```

   `store_mailbox_key` writes the raw 32-byte seed, not the z32 text.
   `MailboxSecretKey::seed` is `pub(crate)`, so add to
   `crates/mailbox_format/src/keys.rs` a `#[must_use] pub fn to_seed(&self)
   -> Zeroizing<[u8; 32]>` (the deploy wire form stays z32). `mailbox_key`
   reads the row, checks it is 32 bytes (else `Err`, same shape as
   `MasterVault::get`, `app_supervisor/src/keys.rs:214-224`), and returns
   `MailboxSecretKey::from_seed`. All three open the store with
   `self.storage_provider.open_service_db(service_id, &self.key_store)`
   (check the field names on `ControlPlaneService`; `dispatch.rs:126-130`
   shows the call).
2. Admission: `deploy/admission.rs:93-113`. Replace the tuple return of
   `validate_deploy_certs_and_visibility` with a struct (in
   `orchestration/types.rs`, next to `PreparedAppContext`):

```rust
pub(crate) struct ValidatedDeploy {
    pub(super) instance_cert: Option<DelegationCertificate>,
    pub(super) visibility: AppVisibility,
    pub(super) mailbox_key: Option<MailboxSecretKey>,
}
```

   and at the end of the function call a new free function in
   `types.rs` (next to `validate_publication`, so it is unit-testable
   without a service):

```rust
/// A mailbox needs a public record that names it with this key, and an
/// instance certificate the receiver can show at registration.
pub(crate) fn validate_mailbox(
    service_id: &str,
    credentials: Option<&MailboxCredentials>,
    visibility: AppVisibility,
    registry_certificate: Option<&str>,
    has_instance_certificate: bool,
) -> Result<Option<MailboxSecretKey>, String>
```

   pseudo-code:

```text
record_mailbox = registry_certificate
    .map(parse SignedEndpointInfo)            // already parsed ok by validate_publication
    .and_then(|s| s.info.mailbox)
match (credentials, record_mailbox):
  (None, None)        -> Ok(None)
  (None, Some(_))     -> Err("record of '{service_id}' names a mailbox, but no mailbox key was supplied; the service could not open its messages")
  (Some(_), None)     -> Err("a mailbox key was supplied for '{service_id}', but its record names no mailbox")
  (Some(c), Some(m))  ->
      if visibility != Public -> Err("only a public service can have a mailbox ('{service_id}' is '{v}')")
      if !has_instance_certificate -> Err("a service with a mailbox needs an instance certificate")
      key = MailboxSecretKey::from_z32(&c.secret_key)
              .map_err(|e| format!("mailbox key for '{service_id}' does not parse: {e}"))?
      if key.public_key() != m.key -> Err("mailbox key for '{service_id}' does not match the key in its record")
      Ok(Some(key))
```

   Never put the secret text in an error string. `(Some, Some)` with
   visibility `Private` cannot reach here (a private deploy has no registry
   certificate), but keep the check: it states the rule.
3. Call sites of `validate_deploy_certs_and_visibility`: only
   `deploy.rs:23-24`. Change to
   `let validated = self.validate_deploy_certs_and_visibility(...)?;`
   and use `validated.instance_cert`, `validated.visibility`,
   `validated.mailbox_key` below. `compute_deploy_idempotency_hash` (C3-7)
   is unchanged.
4. Commit: `deploy/commit.rs:11` `commit_deploy_registry_writes` takes
   `installed_instance_cert` and `validated_visibility` today; pass
   `validated: ValidatedDeploy` in their place (one parameter fewer).
   After the instance-certificate install (:46-61) and before deploy facts,
   add:

```text
match validated.mailbox_key:
  Some(key) => if let Err(e) = self.store_mailbox_key(service_id, &key).await {
                   rollback "mailbox-key installation" (same call as the cert branch)
                   return Err(format!("Mailbox key installation failed: {e}. \
                       With storage encryption on, inject the KEK before deploying a service with a mailbox."))
               }
  None      => self.clear_mailbox_key(service_id).await   // best effort, see its doc
```

   If the function grows past 100 lines, extract the two install blocks
   into helpers (`install_instance_cert`, `install_mailbox_key`); do not
   add a lint suppression.
5. Undeploy: `lifecycle.rs:442-457` `remove_deploy_registry_rows` — add
   `self.clear_mailbox_key(service_id).await;` (best effort, like the other
   removals there). Undeploy keeps the rest of `services/<id>/` as today.
6. Tests (`crates/control_plane/src/service/orchestration/tests/`; add a
   new file `mailbox.rs` and declare it in `tests.rs`; reuse `helpers.rs`):
   - `types.rs` unit tests for `validate_mailbox`: each `Err` case above,
     and the `Ok(Some)` case. Build records with
     `syneroym_sdk`-free helpers: sign an `EndpointInfo` with a test master
     in the test itself, the way `helpers.rs:1004` does.
   - `a_deploy_with_a_mailbox_stores_the_key_in_the_vault` (then
     `mailbox_key()` returns an equal public key).
   - `a_redeploy_without_a_mailbox_clears_the_key`.
   - `undeploy_clears_the_mailbox_key`.
   - `a_deploy_with_a_mismatched_mailbox_key_is_refused` and that nothing
     was stored.
   - `a_redeploy_that_changes_only_the_mailbox_is_not_a_noop` (hash).
   - `the_same_member_on_two_substrates_stores_the_same_key`: two
     `ControlPlaneService` instances (two node identities), the same master
     record and `derive_from_master` key, `mailbox_key()` equal (K-2).

### Step 6 — per-service settings in the app model (`crates/app_orchestration`)

1. `models/service.rs` `ServiceSpec` (:298-343), after `topology_visibility`:

```rust
    /// The mailbox service of every member of this service (ADR-0026).
    /// Each member still has its own mailbox key, derived from its own
    /// master. Only a `public` service may have one. Lives here and on
    /// `PlannedService`, not in `ServiceConfig`: the substrate learns the
    /// mailbox from the signed record, not from its deploy manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox: Option<ServiceId>,
    /// Lifetime of each issued instance certificate, in hours, for every
    /// member of this service. Absent means the deployer's default. A long
    /// value suits a substrate that sleeps (a phone); revocation stays the
    /// main control. 1 to 720.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_cert_lifetime_hours: Option<u32>,
```

   Add a constant in the same file:
   `pub const MAX_INSTANCE_CERT_LIFETIME_HOURS: u32 = 720;` with a doc line
   that it equals the substrate's install limit (30 days).
2. `models/plan.rs` `PlannedService` (:22-73): the same two fields, with
   docs "Cloned from `ServiceSpec.<field>`; the supervisor holds no
   manifest, so the plan is where it reads it."
3. `compiler.rs:226-245`: copy both fields into each `PlannedService`.
4. Compiler validation: next to `validate_plan_visibility`
   (`compiler.rs:67-112`), add a check over the manifest services:
   - `mailbox.is_some()` and `config.visibility != Visibility::Public` →
     error naming the service and its visibility.
   - `instance_cert_lifetime_hours` outside `1..=MAX_INSTANCE_CERT_LIFETIME_HOURS`
     → error naming the service and the range.
   Call it wherever `validate_plan_visibility` is called.
5. **Every `ServiceSpec { .. }` and `PlannedService { .. }` literal gets
   `mailbox: None, instance_cert_lifetime_hours: None`.** `ServiceSpec`:
   about 23 literals in 14 files (`grep -rn 'ServiceSpec {' crates apps`).
   `PlannedService`: about 37 in 17 files (`grep -rn 'PlannedService {' crates apps`).
6. Tests in `crates/app_orchestration/src/compiler/tests.rs`:
   - both fields reach every member of a `replicas = 2` service;
   - a manifest without them compiles to a plan whose JSON has neither key;
   - `mailbox` on an `internal` and on a `private` service is refused;
   - lifetime `0` and `721` are refused, `720` is accepted;
   - a TOML manifest with `mailbox = "did:key:..."` and
     `instance_cert_lifetime_hours = 168` parses (in `models/tests.rs`).

### Step 7 — SDK: records, certificates and keys per member (`crates/sdk`)

1. `crates/sdk/src/deploy/types.rs`: add

```rust
/// What `certify_placed_members` made for each placed member, keyed by
/// member master `ServiceId`. Empty for a deploy without masters.
#[derive(Debug, Default)]
pub struct CertifiedMembers {
    pub instance_certificates: BTreeMap<ServiceId, String>,
    pub registry_certificates: BTreeMap<ServiceId, String>,
    /// Only members whose service names a mailbox.
    pub mailbox_keys: BTreeMap<ServiceId, MailboxSecretKey>,
}
```

   `MailboxSecretKey`'s `Debug` prints only the public key, so the derive
   is safe. `ApplyRequest` (:18-42): replace `instance_certificates` and
   `registry_certificates` with `pub members: &'a CertifiedMembers`.
   `crates/sdk/Cargo.toml`: add `syneroym-mailbox-format.workspace = true`.
2. `crates/sdk/src/deploy/certify.rs`:
   - `member_registry_record` gains a last parameter
     `mailbox: Option<EndpointMailbox>` and sets `mailbox` in the literal.
     Callers: `certify_placed_members` (below) and the tests
     `crates/substrate/tests/{roym_identity_e2e.rs:99,roym_app_e2e.rs:113,multi_substrate_placement_e2e.rs:351}`,
     `crates/substrate/tests/common/roym.rs:149,589`,
     `crates/sdk/src/deploy/tests.rs:115,129,142` — pass `None`.
     It must return `Err` when `visibility != Public` and `mailbox.is_some()`
     (defence in depth for callers that skip the compiler).
   - `certify_placed_members`: replace the parameter `expires_hours: u64`
     with `default_expires_hours: u64` and return `Result<CertifiedMembers>`.
     Per service:

```text
expires_hours = svc.instance_cert_lifetime_hours.map_or(default_expires_hours, u64::from)
cert = certify_instance(client, master, svc.service_id, expires_hours)
mailbox = svc.mailbox.as_ref().map(|mailbox_id| {
    let key = MailboxSecretKey::derive_from_master(master);
    let route = EndpointMailbox { service_id: mailbox_id.to_string(), key: key.public_key() };
    members.mailbox_keys.insert(svc.service_id.clone(), key);
    route
})
record = member_registry_record(visibility, service_id, substrate_id, master, not_after, mailbox)
```

   Update the function doc (it no longer returns a tuple).
   Callers (2): `apps/roymctl/src/commands/member_identity.rs:203` and
   `crates/app_supervisor/src/service/bindings.rs:216` (Steps 8 and 9).
3. `crates/sdk/src/mapper.rs`:
   - `map_deployment_plan_to_wit` (:427): replace the two map parameters
     with `members: &CertifiedMembers`; `PlanMappingContext` gets
     `members: &'a CertifiedMembers` in place of its two map fields.
     Callers: `crates/sdk/src/deploy.rs:155-163`,
     `crates/app_supervisor/src/service/bindings.rs` (one call; find with
     `grep -n map_deployment_plan_to_wit`), `crates/sdk/src/mapper/tests.rs` (3).
   - `map_planned_service` (:381-401): set

```rust
mailbox: ctx.members.mailbox_keys.get(&svc.service_id).map(|key| MailboxCredentials {
    secret_key: key.to_z32().to_string(),
}),
```

     The `String` inside `MailboxCredentials` is not zeroized (it is a
     generated type); the deploy call serializes it at once. Note this in
     the backlog row of §5, not in code.
4. `crates/sdk/src/deploy.rs` `apply_plan` (:116-185): pass `req.members`.
5. **Every `ApplyRequest { .. }` literal** (20; `grep -rn 'ApplyRequest {' crates apps`):
   replace the two certificate fields with `members: &members` (or
   `&CertifiedMembers::default()` where both maps were empty).
6. Tests in `crates/sdk/src/deploy/tests.rs` and `mapper/tests.rs`:
   - `member_registry_record` with a mailbox writes the field; with a
     mailbox and `Internal` it errs.
   - `map_planned_service` puts the z32 key into `manifest.mailbox` only
     for members in `mailbox_keys`.
   - The per-service lifetime is used: a pure test of the
     `expires_hours` choice. If `certify_placed_members` needs a live
     client, extract the choice into
     `fn member_cert_hours(svc: &PlannedService, default: u64) -> u64` and
     test that.

### Step 8 — roymctl (`apps/roymctl`)

**`svc deploy`** (`commands/svc.rs:47-152`, dispatch :218-260,
`svc/deploy.rs:303-413`):

1. New args in `SvcCommands::Deploy`:

```rust
        /// Mailbox service for this service (ADR-0026). Needs
        /// `--visibility public` and `--master` or `--identity`.
        #[arg(long, value_parser = parse_mailbox_choice)]
        mailbox: Option<ServiceId>,
        /// Lifetime of the instance certificate `--master` issues, in
        /// hours (1 to 720). Use a long value for a substrate that sleeps.
        #[arg(long, default_value_t = 24, conflicts_with = "instance_certificate",
              value_parser = clap::value_parser!(u32).range(1..=720))]
        cert_lifetime_hours: u32,
```

   `parse_mailbox_choice(s: &str) -> Result<ServiceId, String>`: `"auto"` →
   the C3-14 message; else `ServiceId::try_new(s)` with its error text.
   Put it in `commands/svc/deploy.rs` (or a small shared
   `commands/mailbox_arg.rs` if `app deploy` uses it too — it does, so
   make it shared: `apps/roymctl/src/commands/mailbox_arg.rs`, declared in
   `commands.rs`).
2. `handle_deploy` already takes 20 parameters with
   `#[allow(clippy::too_many_arguments)]`. Do not add two more: group the
   publication-related ones that `build_publication` and
   `resolve_instance_cert` use (`nickname`, `visibility`, `record_out`,
   `master`, `instance_certificate`, plus the new `mailbox` and
   `cert_lifetime_hours`) into a `struct PublicationArgs` built in
   `svc.rs` at the dispatch. If that grows the change too much, add the two
   parameters and keep the existing `allow` — but no new suppression.
3. `require_stated_visibility_if_signable` (:67-86) or a new check right
   after it: `--mailbox` without a signer → error "--mailbox needs
   --master or --identity: the mailbox key is derived from the service's
   master"; `--mailbox` with a visibility other than `public` → error
   (C3-5 reason, shortened).
4. `signed_export_record` (:435-455) gains `mailbox: Option<EndpointMailbox>`
   and sets the field. `build_publication` (:102-154) computes, when
   `--mailbox` is given:
   `key = MailboxSecretKey::derive_from_master(signer)`,
   `route = EndpointMailbox { service_id, key: key.public_key() }`,
   passes `Some(route)` to `signed_export_record`, and returns the key with
   the publication so `handle_deploy` can put it into the manifest. The
   `--record-out` file therefore carries the field (K-7, last sentence).
5. `resolve_instance_cert` (:159-195): use `cert_lifetime_hours` in place
   of the private `DEFAULT_INSTANCE_CERT_EXPIRES_HOURS` const (:25); delete
   the const if nothing else uses it.
6. SDK client: `DeploySvcOptions` (`crates/sdk/src/types.rs`) gains
   `pub mailbox: Option<MailboxSecretKey>`; `deploy_svc_wasm_with_options`
   (`crates/sdk/src/client/orchestrator.rs:54-79`) maps it into
   `DeployManifest.mailbox`. `deploy_svc_tcp` (:97-117) and
   `deploy_container` (:137-163) take no options struct today: give both
   the same `mailbox` handling **only if** their signatures can take it
   without a new positional parameter chain; otherwise make roymctl
   refuse `--mailbox` with `--tcp` and `--image` ("not supported for tcp
   and container services yet") and record that in the backlog (§5).
   Prefer supporting all three; check how `--master` reaches tcp and
   container deploys today and follow the same path.

**`app deploy`** (`commands/app.rs:131-160`, dispatch :257-279,
`app/deploy.rs:609-725`):

7. New arg:

```rust
        /// Mailbox for one service of the manifest, as
        /// `<service-name>=<mailbox-service-id>`. Repeat for more services.
        /// Overrides the manifest's `mailbox` for that service. Needs
        /// `--mint-masters`.
        #[arg(long = "mailbox", value_parser = parse_service_mailbox, requires = "mint_masters")]
        mailboxes: Vec<(String, ServiceId)>,
```

   `parse_service_mailbox` in `commands/mailbox_arg.rs`: split once on
   `=`, empty name → error, value through `parse_mailbox_choice`.
8. Apply the overrides to the parsed manifest **before** compiling: in
   `app/deploy.rs`, where the manifest is loaded or auto-generated
   (:214-272), set `manifest.services[name].mailbox = Some(id)`; an unknown
   name → error listing the manifest's service names. The compiler then
   applies C3-5. A `.wasm` path manifest has one service: its generated
   name is the one to use; say so in the arg help if the name is not
   obvious (read `deploy.rs:214-256`).
9. `member_identity.rs:160-220` `substitute_and_certify_members`: call
   `certify_placed_members(..., DEFAULT_INSTANCE_CERT_EXPIRES_HOURS)` and
   return `(DeploymentPlan, CertifiedMembers)`. Its callers
   (`app/deploy.rs:483-505` `build_deploy_plan`, and the non-mint branch
   that builds empty maps) switch to `CertifiedMembers`
   (`CertifiedMembers::default()` without `--mint-masters`).
10. `ApplyRequest` at `app/deploy.rs:687-708`: `members: &members`.
11. Tests:
    - `apps/roymctl/tests/cli_args.rs`: `svc deploy --help` lists
      `--mailbox` and `--cert-lifetime-hours`; `svc deploy --mailbox auto`
      fails with "not supported yet"; `--mailbox not-a-did` fails;
      `--cert-lifetime-hours 0` and `721` fail; `app deploy --help` lists
      `--mailbox`; `app deploy --mailbox x=did:key:...` without
      `--mint-masters` fails (clap `requires`).
    - `apps/roymctl/src/commands/svc/tests.rs`: `signed_export_record`
      with a mailbox round-trips through `--record-out`
      (extend `a_record_out_file_round_trips_through_new_with_record`);
      `--mailbox` with `--visibility internal` is refused.
    - `apps/roymctl/src/commands/mailbox_arg.rs` sibling tests for both
      parsers.
    - `apps/roymctl/src/commands/app/tests.rs`: an override for an unknown
      service name is refused; an override reaches the compiled plan.

### Step 9 — supervisor (`crates/app_supervisor`)

1. `service/bindings.rs:216-251` `apply_with_clients`: take
   `CertifiedMembers` from `certify_placed_members(plan, masters, clients,
   None, self.renewed_cert_expires_hours)` and pass `members: &members` to
   `ApplyRequest`. Update the one `map_deployment_plan_to_wit` call in
   this file.
2. `service/renewal.rs:145-196` `renew_one_member`: find the member's
   `PlannedService` once (the rotation lookup at :184-188 already does
   `plan.services.iter().find(...)`; move that lookup to the top and reuse
   it), then
   `let hours = svc.and_then(|s| s.instance_cert_lifetime_hours).map_or(self.renewed_cert_expires_hours, u64::from);`
   and pass `hours` to `certify_instance_via_actor`.
3. The node-wide `renewed_cert_expires_hours` test that requires it to be
   below 24 (`crates/core/src/config/tests.rs:105-108`) stays: it is about
   the default. A per-service value above it is allowed by design (C3-9).
4. Tests:
   - `service/tests/renewal.rs`: a member whose plan entry has
     `instance_cert_lifetime_hours = Some(168)` is renewed with a 168-hour
     certificate (read `expires_at_secs - issued_at_secs` from the
     certificate the fake actor received in `renew_cert`); a member
     without it keeps `renewed_cert_expires_hours`.
   - `service/tests/bindings.rs`: an apply of a plan whose service has a
     mailbox sends `manifest.mailbox` and a registry certificate whose
     `info.mailbox.key` equals `derive_from_master(master).public_key()`
     (the fake actor records the `WitDeploymentPlan`).

### Step 10 — sender: `resolve_peer` and the peer record store (`crates/router`)

1. New file `crates/router/src/peer_records.rs` (declare `pub mod
   peer_records;` in `crates/router/src/lib.rs`), tests in
   `crates/router/src/peer_records/tests.rs`.

```rust
/// The last signed service record seen for each peer that has a mailbox,
/// kept on disk until the record's `not_after`. A sender reads it when the
/// registry cannot be reached or no longer holds the record (a receiver
/// that sleeps for hours has none there), so it can still find the
/// receiver's mailbox.
pub struct PeerRecordStore {
    conn: Arc<Mutex<Connection>>,            // std::sync::Mutex, used only in spawn_blocking
    known: DashMap<String, Known>,           // service_id -> what the row holds
}

struct Known {
    signed_at: u64,                          // pkarr timestamp of the stored record
    packet_hex: String,                      // pkarr_packet_hex of the stored record
}

impl PeerRecordStore {
    pub fn open(path: &Path, now_secs: u64) -> anyhow::Result<Self>;
    pub fn open_in_memory() -> anyhow::Result<Self>;
    pub async fn remember(&self, record: &SignedEndpointInfo) -> anyhow::Result<()>;
    pub async fn recall(&self, service_id: &str) -> Option<SignedEndpointInfo>;
}
```

   Schema (run on every open, `CREATE TABLE IF NOT EXISTS`, WAL mode,
   parent directory created like `registry_store.rs:19-35`):

```sql
CREATE TABLE IF NOT EXISTS peer_records (
    service_id  TEXT PRIMARY KEY,
    record_json TEXT NOT NULL,     -- serde_json of SignedEndpointInfo
    signed_at   INTEGER NOT NULL,  -- pkarr timestamp, microseconds
    not_after   INTEGER NOT NULL
);
```

   `open`: create the table, `DELETE FROM peer_records WHERE not_after < now`,
   then load `(service_id, signed_at, record_json)` into `known` (the
   `pkarr_packet_hex` of the parsed record; delete a row that does not
   parse).

   `remember` pseudo-code:

```text
if record.info.endpoint_type != Service: return Ok
if known[id].packet_hex == record.pkarr_packet_hex: return Ok   // the common case: no work
if record.info.mailbox.is_none() and !known.contains(id): return Ok   // peer without a mailbox: no work
signed_at = record.verify()?.as_u64()                       // check the pkarr Timestamp accessor in the cached crate source
if known[id].signed_at >= signed_at: return Ok             // never replace a newer record
if record.info.mailbox.is_none():
    spawn_blocking(DELETE row id); known.remove(id)         // the deployer removed the mailbox
else:
    spawn_blocking(INSERT OR REPLACE row); known.insert(id, Known { signed_at, packet_hex })
```

   `recall`: `spawn_blocking` read of the row; parse; `verify()` (checks the
   signature and `not_after`); on failure delete the row and its `known`
   entry and return `None`.

   `rusqlite` (`crates/router/Cargo.toml:61`) and `dashmap` (:31) are
   already router dependencies. Add no new dependency for this. Keeping the
   packet text in memory costs about 1 KB per peer with a mailbox.
2. `crates/router/src/net_iroh.rs`: add

```rust
/// What a sender learns about a peer from its records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPeer {
    /// How to reach the peer now. `None` when its substrate record cannot
    /// be found, which is normal for a receiver that sleeps.
    pub addr: Option<EndpointAddr>,
    /// The peer's mailbox, from its own signed record.
    pub mailbox: Option<EndpointMailbox>,
}

/// Resolves a service or substrate DID. Unlike a full lookup, it keeps the
/// service record's mailbox, and a missing substrate record is not an
/// error when the service has a mailbox: the sender can still deposit.
pub async fn resolve_peer(
    registry_client: &RegistryClient,
    peer_records: Option<&PeerRecordStore>,
    service_id: &str,
) -> anyhow::Result<ResolvedPeer>
```

   pseudo-code:

```text
record = match registry_client.lookup(service_id, false).await {
    Ok(r)  => { if let Some(s) = peer_records && let Err(e) = s.remember(&r).await { warn!(..) }; r }
    Err(e) => match peer_records {
        // `lookup` gives the same error for "registry down" and "not
        // found"; a stored record is the right answer to both.
        Some(s) => s.recall(service_id).await.ok_or(e)?,
        None    => return Err(e),
    },
}
mailbox = record.info.mailbox.clone()
mechanisms = match record.info.endpoint_type {
    Substrate => record.info.mechanisms,
    Service   => match registry_client.lookup(&record.info.substrate_id, false).await {
        Ok(sub)                     => sub.info.mechanisms,
        Err(e) if mailbox.is_some() => { debug!(..); Vec::new() }
        Err(e)                      => return Err(e),
    },
}
addr = iroh_addr_from_mechanisms(mechanisms)?   // Option; Err only for a bad Iroh entry
Ok(ResolvedPeer { addr, mailbox })
```

   `iroh_addr_from_mechanisms(Vec<EndpointMechanism>) -> anyhow::Result<Option<EndpointAddr>>`
   is the current loop body of `resolve_iroh_addr` (:158-170) moved into a
   private function. `resolve_iroh_addr` becomes:

```rust
pub async fn resolve_iroh_addr(registry_client: &RegistryClient, service_id: &str)
    -> anyhow::Result<EndpointAddr> {
    resolve_peer(registry_client, None, service_id).await?.addr.ok_or_else(|| {
        anyhow::anyhow!("No valid Iroh mechanism found for service '{service_id}' in registry")
    })
}
```

   Behaviour check: with no store and no mailbox, every path gives the
   same `Ok`/`Err` as today (the only difference is the error text when
   the substrate record is missing). Callers of `resolve_iroh_addr` need
   no change: `proxy/router.rs:301` (changed below anyway),
   `route_handler/io.rs:484`, `crates/substrate/tests/master_endpoint_record_e2e.rs:242,245,329,332`,
   `multi_substrate_placement_e2e.rs:499,502,573` (only `.id` and
   `expect_err` are checked).
   Keep `net_iroh.rs` under 800 lines (it is 245 now).
3. `crates/router/src/proxy.rs`: field
   `peer_records: Option<Arc<PeerRecordStore>>` (default `None` in `new`),
   builder

```rust
    /// The store `invoke_remote` keeps peer records in, so a peer that
    /// cannot be reached can still be found by its mailbox.
    #[must_use]
    pub fn with_peer_records(mut self, store: Arc<PeerRecordStore>) -> Self
```

   placed with `with_outbox` (:219).
4. `crates/router/src/proxy/router.rs:300-305` `invoke_remote`:

```rust
let peer = net_iroh::resolve_peer(&self.registry_client, self.peer_records.as_deref(), &req.target_service)
    .await
    .map_err(|_| ProxyError::ServiceNotFound(req.target_service.clone()))?;
let Some(addr) = peer.addr else {
    return Err(ProxyError::ServiceNotFound(req.target_service.clone()));
};
self.invoke_remote_at(&addr, req).await
```

   `peer.mailbox` is not used yet. Do not write a comment that names a
   later slice; if a comment is needed, say "a peer with a mailbox but no
   address cannot be called directly".
5. `crates/router/src/route_handler.rs:229` `build_proxy`: after the
   `ProxyRouter::new(..)` at :287,

```text
match PeerRecordStore::open(&config.storage.db_dir.join("peer_records.db"), now_secs()) {
    Ok(store) => proxy = proxy.with_peer_records(Arc::new(store)),
    Err(e)    => warn!("peer record store unavailable, offline peers cannot be found by mailbox: {e}"),
}
```

   Use the existing "now in seconds" helper of the router if there is one
   (`grep -rn 'fn now_secs\|fn unix_now' crates/router/src`).
6. Tests:
   - `peer_records/tests.rs` (pure, `open_in_memory` or a temp dir):
     a record with a mailbox is recalled; a record without a mailbox is not
     stored; an older record does not replace a newer one; a newer record
     without a mailbox deletes the stored one; an expired record is not
     recalled and its row is gone; the store survives reopening the file;
     a substrate record is ignored; a record that does not verify is not
     stored.
   - `crates/router/src/net_iroh.rs` tests (or a new
     `net_iroh/tests.rs` if the inline module gets long — then move the
     existing inline tests there too, per the module layout rule):
     `resolve_peer` with an unreachable registry
     (`RegistryClient::new(false, Some(url))` where `url` is
     `http://127.0.0.1:<port>` from a `TcpListener::bind("127.0.0.1:0")`
     that is dropped before the call) returns the stored record's mailbox
     and `addr: None`; with no store it errs.
   - Substrate integration test (new file
     `crates/substrate/tests/mailbox_record_e2e.rs`, using
     `SubstrateTestContext` / `common::alloc_ports`, nothing hard-coded):
     1. Boot one node with a community registry role.
     2. Deploy a public service with `--mailbox`-equivalent SDK options
        and a master (reuse the `svc deploy` helpers in `tests/common`);
        a registry lookup shows `info.mailbox.key ==
        derive_from_master(master).public_key()` (K-1, K-2).
     3. Register a second **service** record by hand, signed by a new
        master, with a mailbox and a `substrate_id` of a DID that has no
        record. `resolve_peer(&client, None, that_id)` returns
        `addr: None` and the mailbox; `resolve_iroh_addr` errs (K-5).
     4. A deploy of a `private` service with a mailbox key is refused
        (K-7).
     5. `--cert-lifetime-hours 168`-equivalent deploy: the service's
        status reports `instance_certificate_expires_at - issued_at ==
        168 * 3600` (K-8).

### Step 11 — docs in the same PR

1. `docs/planning/changes/service-mailbox/change.md`:
   - `## Deviations`: add `D-C3-1` to `D-C3-n`, one line each:
     derived key (C3-1); the deploy field and its record (C3-2); the
     reserved vault name (C3-3); public only (C3-5); the deploy-time pair
     check (C3-6); per-service, not per-member, settings and the 1-720
     range (C3-8, C3-9); `resolve_peer` beside an unchanged
     `resolve_iroh_addr` (C3-11); the store keeps only records with a
     mailbox, plaintext (C3-12); "relocation" means any later deploy of the
     member, because the supervisor does not relocate (§6 item 1).
   - Design "Mailbox key": replace the "*Unverified:*" sentence with the
     answer (deploy field `mailbox`, service vault under a reserved name).
   - `## Open questions`: remove the first item (answered); keep the rest.
   - K-7: change the last sentence to "A public record exported with
     `--record-out` carries the field like any other record." The Design
     paragraph "Private and internal services" stays as it is.
   - `## Tasks`: tick "Record field, mailbox key creation and delivery,
     per-member certificate lifetime, `roymctl svc deploy` and `app deploy`
     options" (note "`auto` waits for the registry list" after it), and in
     "Lookup change" tick the first two parts; the third part (mailbox
     republishes) stays open for C4.
   - Fill the `Test` column for K-1, K-2, K-5, K-6, K-7, K-8 with the test
     names. K-3 gets the CLI tests and "`auto`: open". K-4 stays empty
     (C4).
2. `task.md`: status line (C3 done, date); under C3 add a "Done <date>.
   Decisions `D-C3-1` to `D-C3-n` are in the mailbox change doc." line;
   in §5 the row "The deploy call changes twice ..." says "designed in
   S0"; change it to "designed in C3 (`mailbox-credentials` record)". Add
   to C4: "the mailbox service id in a record is a `did:key` resolvable by
   an ordinary lookup (decide whether it is the host node DID, see the
   `substrate:<node did>/mailbox` resource of ADR-0015 A10)". Add to C6:
   "read the key with `ControlPlaneService::mailbox_key`; pull only for a
   service whose stored record names a mailbox **and** whose key is
   present; the vault needs the KEK, so the pull waits for it after a
   restart". Add to C8: "add `child-token: option<string>` to the WIT
   record `mailbox-credentials`; the only builders are `map_planned_service`
   and `deploy_svc_*` in `crates/sdk`".
3. `status.md`: C3 row with evidence (test files and counts, verify
   result), like the C2 row.
4. Living docs (the AGENTS.md close-out): 
   - `docs/system-architecture.md`: the service record section (new
     optional `mailbox` field); the deploy manifest (new `mailbox` field,
     vault reserved names); the guest vault interface (names starting with
     `syneroym.` are not visible); the sender lookup (stored peer records
     in `peer_records.db`).
   - `docs/developer-guide.md`: the manifest fields `mailbox` and
     `instance_cert_lifetime_hours`; the new roymctl flags; the new file
     `peer_records.db` under `storage.db_dir`.
   - `docs/TERMINOLOGY.md`: "mailbox key" (derived from the member master).
   - The user guide for roymctl, if one exists for `svc deploy` / `app
     deploy` (`grep -rln 'record-out' docs`): document both flags.
   Add each file you touch to `living-docs-touched` in the change doc's
   front matter. Keep `status: approved` (the change is not done until C8).
5. Deferred backlog (`docs/planning/deferred-backlog.md`), see §5.

## 4. Test list (summary)

| Criterion | Tests (Step) |
| --- | --- |
| K-1 | core `dht_registry/tests.rs` (2); SDK record tests (7); e2e step 2 (10) |
| K-2 | `keys/tests.rs` derivation tests (1); control-plane `mailbox.rs` incl. two substrates (5); supervisor bindings test (9); e2e step 2 (10) |
| K-3 | `cli_args.rs`, `svc/tests.rs`, `app/tests.rs`, `mailbox_arg` tests (8). `auto`: open until C7 |
| K-5 | `net_iroh` tests and e2e step 3 (10) |
| K-6 | `peer_records/tests.rs` and the unreachable-registry `resolve_peer` test (10) |
| K-7 | compiler tests (6); `validate_mailbox` tests (5); roymctl refusal (8); e2e step 4 (10) |
| K-8 | compiler range tests (6); lifetime choice (7); supervisor renewal (9); e2e step 5 (10) |
| Vault safety | `delete_secret`, `is_reserved`, guest `reveal` refused on both hosts, `set-secret` refused (4) |

## 5. Deferred-backlog rows to add (Step 11)

Each with reason, target and a link to the code:

1. **The deploy manifest's mailbox key is a plain `String` in generated
   types.** `wit-bindgen` derives `Debug` on `DeployManifest` and
   `MailboxCredentials`, so a `{:?}` of a manifest would print the key, and
   the string is not zeroized. No code logs a manifest today. Target: TBD.
   Link: `control-plane.wit` (`mailbox-credentials`), `crates/sdk/src/mapper.rs`.
2. **`peer_records.db` is plaintext.** It lists the peers with a mailbox
   that this node called. Records are public, the list is not. Encrypting
   it needs the KEK at boot. Target: TBD. Link: `crates/router/src/peer_records.rs`.
3. **Certificate lifetime and mailbox are per service, not per member.**
   All members of a replicated service share both values; a member on a
   phone and a member on a server cannot differ. Target: TBD. Link:
   `crates/app_orchestration/src/models/service.rs` (`ServiceSpec`).
4. **A mailbox deploy needs the vault unlocked.** With storage encryption
   on, a deploy with a mailbox fails until the operator injects the KEK.
   Target: TBD (same as the other KEK-at-boot rows; link them).
5. Only if Step 8.6 ends with a refusal: **`svc deploy --mailbox` is not
   supported for tcp and container services.**
6. **A mailbox for a private or internal service.** A mailbox does not need
   the registry: a sender needs only the mailbox id and the key, which an
   explicitly given signed record carries (like a fixed relay URL). Not
   built because: (a) the substrate learns which mailbox to pull from out
   of the stored public record, so the WIT record `mailbox-credentials`
   would need a `service-id` field (a small change: only its builders in
   `crates/sdk` change); (b) the mailbox must never republish a record
   with `is_private = true` (K-4), and must get no record from a private
   service at `register`; (c) a sender needs a way to import a peer's
   record (for example into the peer record store); how is an open
   question; (d) ADR-0026 Decision 1 and K-7 need an amendment. Target:
   TBD. Link: `control-plane.wit` (`mailbox-credentials`),
   `crates/router/src/peer_records.rs`, ADR-0026.

Also update the existing row "Service mailbox: later improvements": key
rotation now means a new HKDF `info` version and a new signed record
(`crates/mailbox_format/src/keys.rs`).

## 6. Things that are ambiguous or stale in task.md and the change docs

1. **"Carried at relocation" (K-2, task.md C3): there is no relocation.**
   The supervisor refuses to move a running member
   (`refuse_placement_change`, `crates/app_supervisor/src/service/verbs/submit.rs:135-178`;
   "relocation is a non-goal", `crates/app_orchestration/src/alerts.rs:57-60`;
   backlog rows at `deferred-backlog.md:368,376`). The only way to move a
   member is undeploy, forget, and deploy again (roymctl or a new
   submission). This plan reads K-2 as "every deploy of the same member,
   on any substrate, gets the same key", which C3-1 gives without any
   relocation code. The change doc's "When the supervisor relocates a
   member, it sends the private half to the new instance" describes code
   that does not exist. **Confirmed by the owner (2026-10-17)**, together
   with the derived key (C3-1).
2. **The deploy field and the key store (task.md C3 "First").** Decided in
   C3-2 and C3-3. The change doc's "kept in the service's key store" names
   no real store; the vault is the only encrypted per-service one, and its
   guest interface had to be closed for reserved names. task.md §5 still
   says the field "is designed in S0"; S0 moved it to C3 (§3 S0, last
   bullet). Step 11 fixes §5.
3. **K-7's last sentence.** "A record exported with `--record-out` carries
   the field like any other record" could be read as allowing a mailbox on
   a `private` service (private + `--record-out` is how a private record
   is exported today, `svc/deploy.rs:102-154`). But the mailbox republishes
   the record (K-4), which would publish a private record, and the change
   doc's Design says private and internal services have no mailbox field.
   This plan allows a mailbox only on `public` (C3-5), and reads the
   sentence as "a public record written with `--record-out` also carries
   the field". **Decided by the owner (2026-10-17):** public only for now;
   private and internal mailboxes are a backlog item (§5 item 6). Step 11
   rewrites the K-7 sentence to say "a public record written with
   `--record-out`".
4. **The instance certificate is stored in plaintext.** `endpoints.db` is
   opened without a key (`crates/data_db/src/registry_store.rs:53`) even
   though the build uses SQLCipher. That is fine for a certificate, and it
   is why the mailbox secret cannot go next to it. Noted only.
5. **"Per-member certificate lifetime" (K-8).** No per-member setting
   exists anywhere: the compiler copies one `ServiceSpec` into every member
   (`compiler.rs:226-245`), and the inventory entries carry no lifetime.
   C3-8 makes it per service. The other choice, a lifetime on the
   substrate inventory entry (it is the substrate that sleeps), needs the
   field on two inventory types (`SupervisorInventoryEntry`,
   `SubstrateEntry`) and does nothing for `svc deploy`. Recorded as a
   backlog row (§5 item 3).
6. **"`resolve_iroh_addr` returns the mailbox field" (task.md C3).**
   Changing its return type would touch the relay path, which never needs
   it. C3-11 adds `resolve_peer` and keeps `resolve_iroh_addr`. Same
   effect for K-5.
7. **"The last signed record it saw for each peer" (K-6).** Read as "each
   peer that has a mailbox" (C3-12). Storing every peer has no use for a
   deposit and grows the file.
8. **`RegistryClient::lookup` cannot tell "registry down" from "not
   found"** (both are the same `anyhow` error, `client.rs:253-254`). The
   fallback to the stored record therefore runs on any error. That is the
   right answer for both cases (K-5 and K-6), so no change to `lookup`.
   Also: `lookup(.., true)` returns a record whose `mechanisms` were
   replaced, so it no longer verifies (`client.rs:263-267`). `resolve_peer`
   does not use `resolve = true`, so the stored copy always verifies.
9. **What the mailbox `service_id` names is decided in C4.** A mailbox is a
   native service under `[roles.mailbox]`; native services use reserved
   names that resolve to the node DID (`SUPERVISOR_RESERVED_SERVICE_ID`).
   ADR-0015 A10 names the resource `substrate:<host node did>/mailbox`, so
   the record's `service_id` is likely the host node DID. C3 only requires
   a `did:key:` id (`ServiceId`). Step 11 adds this to C4 in task.md.
10. **K-4 is in C4, not C3.** The change doc's task line "Lookup change: ...
    Mailbox republishes the record" mixes C3 and C4 work; task.md puts K-4
    in C4. C3 ticks only its own part.
11. **The C2 plan expected `MailboxSecretKey::generate` at deploy**
    (`slice-c2-implementation-plan.md` §5). C3-1 derives instead;
    `generate` stays for tests. Recorded as `D-C3-1`.
12. **`app deploy` "takes a mailbox for each member" (K-3).** Members of one
    service share the manifest entry, so they share the mailbox service
    (each member still has its own key from its own master). The CLI
    override is per service name (Step 8.7).
13. **`mailbox` and `instance_cert_lifetime_hours` need a master.** A plain
    `app deploy` without `--mint-masters` signs no record and issues no
    certificate, so a mailbox cannot work; the clap `requires` makes the
    CLI refuse it, and the compiler does not check it (the supervisor
    always has masters). A manifest `mailbox` with no `--mint-masters` is
    silently unused: Step 8.8 should also refuse that case ("the manifest
    names a mailbox for '<name>'; deploy with --mint-masters").
14. **The record size.** The change doc says the field adds about 140
    bytes and the record stays well inside 1000 bytes. The nickname has no
    length limit in the record type; a long nickname plus a mailbox could
    pass 1000 bytes, and `sign` would then fail at deploy with a pkarr
    error. Step 2's size test pins a realistic case; if it fails, ask the
    owner before adding a nickname limit.

## 7. Completion steps

1. Import clean-up pass over every edited file (types by `use`, functions
   by their parent module, no inline `a::b::c` paths).
2. `cargo +nightly-2026-04-06 fmt --all`.
3. `cargo clippy -q --workspace --all-targets --all-features -- -D warnings`.
4. Targeted tests while working:
   `cargo nextest run -p syneroym-mailbox-format -p syneroym-core -p syneroym-data-db -p syneroym-control-plane -p syneroym-app-orchestration -p syneroym-sdk -p syneroym-router -p syneroym-app-supervisor -p roymctl`
   then
   `cargo nextest run -p syneroym-substrate --test mailbox_record_e2e`.
5. The WASM parity check: if any `roym_*` crate or guest fixture changed
   (it should not), rebuild with `mise run build:roym`.
6. `cargo shear`, `cargo deny check licenses`, `cargo audit` (sandbox off).
7. The duplication recipe from `AGENTS.md`; lower
   `MAX_EXACT_DUPLICATE_PERCENT` if the gate says the number improved.
   Lower `MAX_TOO_MANY_LINES_SUPPRESSIONS` or the oversized-file list if a
   gate says so.
8. Docs from Step 11, backlog rows from §5.
9. `mise run verify` as a background job, no `--skip`. All gates must pass.
10. Commit on `feat/m06d-c3-implementation`, for example
    `feat(mailbox): add mailbox record field and key`, body of one or two
    sentences. Open the PR with the CLA line from `AGENTS.md`.
