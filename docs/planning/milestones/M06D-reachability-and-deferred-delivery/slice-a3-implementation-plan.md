# M06D slice A3 — Coordinator info: implementation plan

*Reader: the developer (or AI) who implements slice A3. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  (no "M06D", "A3", "B2", "X7", "AC-6", "AC-17" in code, comments or test
  names; ADR ids are fine), simple English in comments and docs, the 800-line
  file cap, the 100-line function cap, and the module layout rule (`foo.rs`
  beside `foo/`, never `mod.rs` under `src/`).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 (row X7), §3 (slices A3, A4 and B2). B2 adds passport routes to the same
  info server later; do not build them.
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-6,
  AC-17, the Tasks list, the "Note for Slice A4" lines and `## Deviations`.
- `docs/planning/changes/operator-passport/change.md`: "Operator root" and
  "Endpoints" (what B2 adds to the info server).
- This plan, §2.

**Branch.** Work on `feat/m06d-a3-implementation`. A2 is merged into `main`
(#302, commit `83d3ae32`), and this branch is based on that commit: it is
`main` plus the commits that add this plan. Check that no code differs from
`main` before you start:

```bash
git fetch origin main
git diff --stat origin/main HEAD -- crates apps xtask
```

The diff must be empty. If `main` has moved on, rebase first
(`git rebase origin/main`). The branch was rebased after it was pushed, so the
first push needs `git push --force-with-lease -u origin
feat/m06d-a3-implementation`. Do not open a pull request unless the user
asks. Commit messages follow the Conventional Commits and 50/72 rules in
`AGENTS.md`.

### Terms used in this plan

- **Info server.** The small HTTP (or HTTPS, when `[tls]` is set) server of an
  Iroh coordinator that serves `GET /v1/info`. It has its own bind address
  (`info_http_bind_address`, or the relay port plus 10). It is built in
  `crates/coordinator_iroh/src/coordinator.rs` (`spawn_http_info_server`).
- **Relay connection.** An open connection from an Iroh endpoint to the relay
  server of this coordinator. One endpoint can hold more than one for a short
  time (see step 3b).
- **Router connection.** A Syneroym connection (ALPN `SYNEROYM_ALPN`) to the
  coordinator's own endpoint. `max_connections` caps these. They are not
  relay connections.
- **Coordinator DID.** `did:key` of the coordinator endpoint's Iroh key. The
  key is made at random at every start (`build_iroh_endpoint`), so this DID
  changes at every start. The coordinator registers its endpoint record in the
  community registry under this DID.
- **Host substrate.** The substrate process that runs the coordinator role. Its
  node DID and its controller are the passport root (operator-passport change
  doc, "Operator root"). The coordinator code does not know them today.

## 1. What this slice delivers

1. `crates/core`: a new module `coordinator_info` with the `/v1/info` wire
   types, moved from `crates/coordinator_iroh`. `status` gets a typed enum.
   New fields: `relay.clients`, `relay.max_clients`, and a new `registry`
   object. A4 (in `crates/substrate`) reads this answer, and
   `crates/substrate` reaches `crates/coordinator_iroh` only behind the
   `coordinator_iroh` feature. So the wire types must live in `crates/core`.
2. `crates/core` config: `[roles.coordinator.iroh] max_relay_clients`.
3. `crates/coordinator_iroh`:
   - `/v1/info` reports the number of relay connections (`relay.clients`),
     computed as `accepts - disconnects` of the `iroh-relay` server counters.
   - `status` is `at_capacity` when `relay.clients >= max_relay_clients`.
     Router connections no longer decide `status` (AC-6).
   - The handler does no network call (AC-17). The registry lookup in the
     handler is removed.
   - A coordinator that shares itself in a registry registers again every
     hour, so its record no longer expires after 2 hours. `/v1/info` reports
     the result of the last registration under `registry`.
   - `identity` is not reported any more (`null`). Its two values were never
     correct (§2, D-A3-4). B2 fills it with the host substrate's identity.
   - `spawn_http_info_server` takes a ready `axum::Router`, so B2 adds its
     routes without changing this function again.
4. Tests for the server side of AC-6, for AC-17, for the registration
   refresh, and for "router cap does not give `at_capacity`".
5. Living docs, change doc, backlog, `task.md` and status updates.

Not in this slice: the substrate resolver and its probe of `info_url` (A4),
passport routes, the TLS rule and the host identity on the info server (B2),
enforcing the relay client cap (D-A3-2).

## 2. Decisions

Each decision below was reviewed. Record each one in the change doc's
`## Deviations` (completion step 2).

- **D-A3-1. Cap key.** A new key `[roles.coordinator.iroh] max_relay_clients:
  Option<u64>`, default `None` (no cap). AC-6 says how to count, but no key
  set a cap. `max_connections` already caps router connections (the
  `RouteHandler` refuses connection `cap + 1` with `ServiceUnavailable`), so
  reusing it would give one number two meanings.
- **D-A3-2. The cap is reported, not enforced.** `iroh-relay` 0.97 has no
  client limit (`Limits` has only an accept rate and a per-client byte rate).
  The `AccessConfig::Restricted` closure could refuse a new client at the cap,
  but it runs before a new connection of an endpoint replaces its old one, so
  a reconnecting endpoint could be refused because of its own old connection.
  B2 also rewrites this closure for passports. A4 skips an `at_capacity`
  relay.
- **D-A3-3. `max_relay_clients = 0` stops startup.** With the relay on, `0`
  always reports `at_capacity`, which is never what an operator means. The
  error names the setting. When `max_relay_clients` is set and
  `enable_relay = false`, startup logs a warning (the key has no effect).
- **D-A3-4. `identity` is `null` until the host identity is known.** Today
  `identity.did` is the coordinator's Iroh endpoint id (not a DID, and new at
  every start), and `identity.controller_status` comes from a registry lookup
  of that id. The registry keys records by DID and by a `nickname-shorthash`
  alias (`crates/community_registry/src/registry.rs:209-225,311`), and the
  coordinator registers under its DID (`coordinator.rs:457`), so the lookup
  never matches and the value is always `unverified`. The M02 schema
  (`docs/planning/milestones/M02-reliable-node/task.md:501-542`) meant
  `identity.did` to be the node's DID and `controller_status: verified` to
  mean "the controller (master-key delegation) is verified". That is the host
  substrate's identity, which is also the passport root that B2 needs. So
  A3 does not give `identity` a new meaning. It stops sending the two wrong
  values and leaves the field `null`. B2 fills it with the M02 meaning. The
  `IdentityInfo` type moves to `crates/core` unchanged
  (`controller_status: String`), so B2 chooses its values.
- **D-A3-5. Registry state has its own field, `registry`.** `registry` is
  present only when the coordinator registers itself
  (`share_in_registry = true` and `community_registry_url` set). It holds the
  coordinator DID (`did`) and whether the last registration was accepted
  (`registered`). Before the first attempt ends, `registered` is `false`.
- **D-A3-6. Register again on a timer, and drop the lookup.** Today the
  coordinator registers once at startup. The registry removes a record that
  is not refreshed within `DEFAULT_REGISTRY_TTL_SECS` (2 hours,
  `crates/core/src/dht_registry/types.rs:16`), so the record is gone after
  about 2 hours (backlog row "A coordinator registers itself once…"). If the
  registry is down at boot, the coordinator never registers. The new loop:
  register (with the existing `retry_with_backoff` and `config.retry`), set
  `registered`, then sleep `HEARTBEAT_INTERVAL_SECS` (1 hour, the same
  interval substrates use, `types.rs:20`) after a success, or
  `REGISTRATION_RETRY` (60 s) after a failure. Each registration signs a new
  record with a newer timestamp, which the registry accepts and which resets
  its TTL clock (`admit_endpoint`, `registry.rs:258`). A separate lookup would
  only tell what a `2xx` from `/register` already told, so there is none. This
  meets AC-17 in the strongest way: probes cause no lookups at all, and a slow
  registry cannot slow a probe (A4 probes with a timeout).
- **D-A3-7. Registration uses `RegistryClient::register`.** Today
  `register_coordinator_in_registry` posts with `reqwest::Client::new()`,
  which has no timeout. In a loop, one hung request would stop the refresh
  forever. `RegistryClient::new(false, Some(url))` has the shared
  `registry_http::HTTP_REQUEST_TIMEOUT` (10 s) and its `register` posts to
  the same `{url}/register`. `false` turns the DHT off: the coordinator has
  never published to the DHT, and this slice does not change that.
- **D-A3-8. `status` reads relay connections only.** A coordinator with no
  relay server, or with no `max_relay_clients`, reports `healthy`. Reaching
  `max_connections` no longer gives `at_capacity`. `connections.active` and
  `connections.cap` stay for operators. A4 must also check `relay.online`
  (completion step 2 adds the note).
- **D-A3-9. The count is connections, not endpoints.** Each client actor
  counts one accept when it starts and one disconnect when it ends
  (`iroh-relay-0.97.0/src/server/client.rs:276-294`). When an endpoint
  connects again, `Clients::register` moves its old client to `inactive` and
  sends it a health message (`server/clients.rs:82-94`). The old actor keeps
  running, and stays counted, until its own connection closes. So for a short
  time one endpoint can count twice.
- **D-A3-10. The coordinator's own endpoint is counted only when its relay
  admits it.** With no parent relay, the coordinator's endpoint uses its own
  relay as its home relay. With `access = "everyone"` it is admitted and
  counted. With `access = [ids]` it is refused, because its id is random at
  every start and cannot be in the list (`restricted_to`,
  `crates/coordinator_iroh/src/config.rs:54`), so it is not counted (and has
  no home relay). With B2's `access = "passport"` it would be refused too,
  because it holds no lease. B2 must decide whether a coordinator's own
  endpoint is always admitted on its own relay (completion step 4 adds the
  note).
- **D-A3-11. Wire changes.** All existing field names stay. `status` becomes
  an enum that serialises to the same strings (`healthy`, `at_capacity`); an
  unknown string reads as `Unknown` (`#[serde(other)]`), so a newer peer
  still parses. `relay` gains `clients` and `max_clients`. New `registry`.
  `identity` is `null` (D-A3-4). `relay.online` keeps its meaning, "this
  coordinator runs a relay server".
- **D-A3-12. Wire types move to `crates/core`.** Reason in §1 item 1. The
  path `syneroym_coordinator_iroh::CoordinatorInfo` goes away; its three users
  change (step 4).

## 3. Step-by-step changes

Files touched:

```text
crates/core/src/lib.rs                                add `pub mod coordinator_info;`
crates/core/src/coordinator_info.rs                   new: wire types (step 1)
crates/core/src/coordinator_info/tests.rs             new: serde tests (step 7)
crates/core/src/config/roles.rs                       new key (step 2)
crates/substrate/config.sample.toml                   new key, commented (step 2)
crates/coordinator_iroh/Cargo.toml                    dev-dep tokio test-util (step 7)
crates/coordinator_iroh/src/lib.rs                    modules, exports (step 4)
crates/coordinator_iroh/src/config.rs                 cap check (step 2)
crates/coordinator_iroh/src/info_endpoint.rs          handler and count (step 3)
crates/coordinator_iroh/src/info_endpoint/tests.rs    new: unit tests (step 7)
crates/coordinator_iroh/src/registration.rs           new: register loop (step 5)
crates/coordinator_iroh/src/registration/tests.rs     new: unit tests (step 7)
crates/coordinator_iroh/src/coordinator.rs            wiring and refactor (step 6)
crates/coordinator_iroh/tests/common/mod.rs           shared registry helper (step 8)
crates/coordinator_iroh/tests/info_endpoint.rs        new: integration tests (step 8)
crates/coordinator_iroh/tests/startup_config.rs       cap `0` test (step 8)
crates/coordinator_iroh/tests/connection_limit.rs     import, router-cap check (steps 4, 8)
crates/coordinator_iroh/tests/multi_hop_relay.rs      import, helper move (steps 4, 8)
crates/coordinator_iroh/tests/tls_rotation.rs         `INFO_PATH` (step 4)
crates/smoke-tests/src/main.rs                        import, `{:?}` (step 4)
xtask/oversized-test-files.txt                        multi_hop_relay.rs count (step 8)
```

### Step 1 — `crates/core/src/coordinator_info.rs` (new)

Add `pub mod coordinator_info;` to `crates/core/src/lib.rs` (keep the list
sorted: after `config`, before `curated_list`).

Move the five structs out of `crates/coordinator_iroh/src/info_endpoint.rs`
(l.19-63) into this file. Change them as below. Keep the field order and
names. Remove `default_status()`.

```rust
//! The answer of `GET /v1/info` on an Iroh coordinator. A substrate reads it
//! before it uses a relay from a curated list, and operators read it to see
//! the coordinator's health.

use serde::{Deserialize, Serialize};

/// The path of the info endpoint on the coordinator's info server.
pub const INFO_PATH: &str = "/v1/info";

/// Whether the coordinator's relay takes more clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorStatus {
    Healthy,
    /// The relay has at least `max_relay_clients` connections. A substrate
    /// picks another relay.
    AtCapacity,
    /// A value this build does not know, or no value.
    #[default]
    #[serde(other)]
    Unknown,
}

/// The identity of the host substrate. Not reported yet: the field is `null`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityInfo {
    pub did: String,
    pub controller_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionsInfo {
    /// Open Syneroym connections to the coordinator's endpoint. Not relay
    /// connections.
    pub active: usize,
    /// `max_connections`.
    pub cap: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsInfo {
    pub cert_expiry_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayInfo {
    /// The coordinator runs a relay server.
    pub online: bool,
    /// Open relay connections now. `None` when there is no relay server.
    #[serde(default)]
    pub clients: Option<u64>,
    /// `max_relay_clients`.
    #[serde(default)]
    pub max_clients: Option<u64>,
}

/// The coordinator's own record in its community registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInfo {
    /// The DID the record is registered under. New at every start.
    pub did: String,
    /// The registry accepted the last registration. Registration repeats
    /// every hour after a success and every minute after a failure.
    pub registered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorInfo {
    pub endpoint_addr_bytes: Vec<u8>, // serde_json-encoded EndpointAddr
    /// The coordinator's Iroh endpoint id. Clients dial it with this id.
    pub substrate_id: String,
    pub relay_url: Option<String>, // Local relay URL hosted by this coordinator
    pub parent_coordinator_url: Option<String>, // Parent coordinator URL

    #[serde(default)]
    pub status: CoordinatorStatus,
    #[serde(default)]
    pub identity: Option<IdentityInfo>,
    #[serde(default)]
    pub connections: Option<ConnectionsInfo>,
    #[serde(default)]
    pub tls: Option<TlsInfo>,
    #[serde(default)]
    pub relay: Option<RelayInfo>,
    /// `None` when the coordinator does not register itself.
    #[serde(default)]
    pub registry: Option<RegistryInfo>,
}
```

Check: `#[serde(other)]` and `#[default]` on the same unit variant compile
together. If not, write `impl Default` by hand.

Add `#[cfg(test)] mod tests;` at the end of the file (tests in step 7).

### Step 2 — the cap: config and startup check

`crates/core/src/config/roles.rs`, in `CoordinatorIrohConfig` (l.297), add
after `max_connections`:

```rust
    /// Relay connections at which `/v1/info` reports `at_capacity`. Reported
    /// only: the relay still admits more clients. `None` means no cap.
    pub max_relay_clients: Option<u64>,
```

In `impl Default for CoordinatorIrohConfig` (l.309) add
`max_relay_clients: None,`. Every struct literal of `CoordinatorIrohConfig`
in the workspace ends with `..Default::default()` (checked: 13 files), so no
other literal changes.

`crates/substrate/config.sample.toml`, in `[roles.coordinator.iroh]`
(l.185-193), add after `quic_bind_address`:

```toml
# Relay connections at which GET /v1/info reports "at_capacity" (reported
# only; the relay still admits more). Leave unset for no cap. Must be >= 1.
# max_relay_clients = 1000
```

`crates/coordinator_iroh/src/config.rs`, next to `relay_access`. Add
`CoordinatorIrohConfig` to its existing import
`use syneroym_core::config::{CoordinatorAccess, CoordinatorRole};` (l.13);
`bail` and `warn` are already imported.

```rust
/// Checks `[roles.coordinator.iroh] max_relay_clients`.
///
/// # Errors
/// For `0`, which would always report `at_capacity`.
pub fn check_relay_cap(iroh_cfg: &CoordinatorIrohConfig) -> Result<()> {
    match iroh_cfg.max_relay_clients {
        Some(0) => bail!("[roles.coordinator.iroh] max_relay_clients must be at least 1"),
        Some(_) if !iroh_cfg.enable_relay => warn!(
            "[roles.coordinator.iroh] max_relay_clients is set, but enable_relay is false; it \
             has no effect"
        ),
        _ => {}
    }
    Ok(())
}
```

Call it in `CoordinatorIroh::init` right after `relay_access(&role.access)?`
(l.106), before the relay server is spawned:
`if let Some(iroh_cfg) = &role.iroh { check_relay_cap(iroh_cfg)?; }`.
A wrong value must stop startup before any listener starts.

### Step 3 — `crates/coordinator_iroh/src/info_endpoint.rs`

After step 1 the file keeps: `InfoState`, `get_cert_expiry_days`, `get_info`.
Add: `relay_client_count`, `relay_status`, `info_router`. The module becomes
crate-private (step 4), so its items are `pub(crate)`.

Imports (types via `use`, functions through their module, per `AGENTS.md`):

```rust
use std::{fs, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

use axum::{Json, Router, extract::State, routing};
use chrono::Utc;
use iroh_relay::server::Metrics as RelayServerMetrics;
use syneroym_core::coordinator_info::{
    ConnectionsInfo, CoordinatorInfo, CoordinatorStatus, INFO_PATH, RegistryInfo, RelayInfo,
    TlsInfo,
};
use tokio::sync::watch;
use x509_parser::{...};   // unchanged
```

Remove the `RegistryClient` import and the `serde` import.

**3a. `InfoState`:**

```rust
#[derive(Debug)]
pub(crate) struct InfoState {
    pub endpoint_addr_bytes: Vec<u8>,
    /// The coordinator's Iroh endpoint id.
    pub substrate_id: String,
    pub relay_url: Option<String>,
    pub parent_coordinator_url: Option<String>,
    pub active_connections: Arc<AtomicUsize>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<PathBuf>,
    /// Counters of the relay server this coordinator runs. `None` when it runs
    /// no relay server.
    pub relay_metrics: Option<Arc<RelayServerMetrics>>,
    pub max_relay_clients: Option<u64>,
    /// The coordinator DID and the result of the last registration. `None`
    /// when the coordinator does not register itself.
    pub registry: Option<(String, watch::Receiver<bool>)>,
}
```

Removed fields: `is_relay_enabled` (now `relay_metrics.is_some()`; the relay
server exists exactly when `enable_relay` is true, `coordinator.rs:123`) and
`registry_client`.

**3b. Relay connection count.** Facts checked in `iroh-relay` 0.97 source:
see D-A3-9. A client refused by the access check never starts an actor
(`server/http_server.rs:750`), so it is not counted. `Counter::get` returns 0
without the `iroh-metrics/metrics` feature; the `iroh-relay` `server` feature
(workspace `Cargo.toml` l.194) turns on `metrics`, so the counters work.

```rust
/// Open relay connections now. `iroh-relay` has no gauge for this; each
/// client actor counts one accept when it starts and one disconnect when it
/// ends. An endpoint that connects again counts twice until its old
/// connection closes.
pub(crate) fn relay_client_count(metrics: &RelayServerMetrics) -> u64 {
    // Read disconnects first: a connection's disconnect is counted after its
    // accept, so this order does not see a disconnect without its accept.
    // The counters are relaxed atomics, so still never go below zero.
    let disconnects = metrics.disconnects.get();
    metrics.accepts.get().saturating_sub(disconnects)
}

pub(crate) fn relay_status(clients: Option<u64>, cap: Option<u64>) -> CoordinatorStatus {
    match (clients, cap) {
        (Some(clients), Some(cap)) if clients >= cap => CoordinatorStatus::AtCapacity,
        _ => CoordinatorStatus::Healthy,
    }
}
```

**3c. Handler.** No network call:

```rust
pub(crate) async fn get_info(State(state): State<Arc<InfoState>>) -> Json<CoordinatorInfo> {
    let relay_clients = state.relay_metrics.as_deref().map(relay_client_count);
    let tls = state
        .tls_cert_path
        .as_deref()
        .filter(|path| path.exists())
        .map(|path| TlsInfo { cert_expiry_days: get_cert_expiry_days(path) });

    Json(CoordinatorInfo {
        endpoint_addr_bytes: state.endpoint_addr_bytes.clone(),
        substrate_id: state.substrate_id.clone(),
        relay_url: state.relay_url.clone(),
        parent_coordinator_url: state.parent_coordinator_url.clone(),
        status: relay_status(relay_clients, state.max_relay_clients),
        identity: None,
        connections: Some(ConnectionsInfo {
            active: state.active_connections.load(Ordering::SeqCst),
            cap: state.max_connections,
        }),
        tls,
        relay: Some(RelayInfo {
            online: state.relay_metrics.is_some(),
            clients: relay_clients,
            max_clients: state.max_relay_clients,
        }),
        registry: state.registry.as_ref().map(|(did, registered)| RegistryInfo {
            did: did.clone(),
            registered: *registered.borrow(),
        }),
    })
}

/// The routes of the info server.
pub(crate) fn info_router(state: InfoState) -> Router {
    Router::new().route(INFO_PATH, routing::get(get_info)).with_state(Arc::new(state))
}
```

Keep `get_info` an `async fn`: axum needs it as a handler.

Add `#[cfg(test)] mod tests;` at the end of the file.

### Step 4 — `lib.rs` and the users of the moved type

`crates/coordinator_iroh/src/lib.rs`:

```rust
mod config;
mod coordinator;
mod info_endpoint;
mod registration;

pub use coordinator::CoordinatorIroh;
```

(Remove `pub mod info_endpoint` and `pub use info_endpoint::CoordinatorInfo`.)

Users (checked with `git grep -n -e CoordinatorInfo -e '/v1/info'`):

| File | Line | Change |
| --- | --- | --- |
| `crates/coordinator_iroh/tests/connection_limit.rs` | 17, 18, 211 | `use syneroym_coordinator_iroh::CoordinatorIroh;`; add `coordinator_info::{CoordinatorInfo, INFO_PATH}` to the `syneroym_core::{...}` import; `format!("http://{info_addr}{INFO_PATH}")` |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs` | 21, 22, 324, 362 | same import change; both URLs use `INFO_PATH` |
| `crates/coordinator_iroh/tests/tls_rotation.rs` | 109 | `format!("https://{info_addr}{INFO_PATH}")`, import `syneroym_core::coordinator_info::INFO_PATH` |
| `crates/smoke-tests/src/main.rs` | 11 | `use syneroym_coordinator_iroh::CoordinatorIroh;` and import `syneroym_core::coordinator_info::CoordinatorInfo` |
| `crates/smoke-tests/src/main.rs` | 166 | `println!("  Status:       {:?}", info.status);` (the enum has no `Display`) |

`crates/smoke-tests/src/main.rs` l.73 and l.122 build the URL from a base URL
given on the command line; leave them, or use `INFO_PATH` there too. No code
reads `info.identity` or compares `info.status` as a string (checked).

### Step 5 — `crates/coordinator_iroh/src/registration.rs` (new)

Move `register_in_global_registry` (`coordinator.rs:327-367`) and
`register_coordinator_in_registry` (`coordinator.rs:446-487`) here, and
change them as below. `coordinator.rs` loses about 90 lines.

```rust
//! The coordinator's own record in its community registry: register, then
//! register again on a timer so the record does not expire.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use iroh::{EndpointAddr, SecretKey};
use syneroym_core::{
    config::RetryPolicy,
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, DEFAULT_REGISTRY_TTL_SECS, EndpointInfo,
        EndpointMechanism, EndpointType, HEARTBEAT_INTERVAL_SECS, RegistryClient,
    },
    retry,
};
use syneroym_identity::{Identity, substrate};
use tokio::{sync::watch, task::JoinHandle, time};
use tracing::{debug, warn};

/// Wait after a successful registration.
pub(crate) const REGISTRATION_REFRESH: Duration = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
/// Wait after a registration that failed all its retries.
pub(crate) const REGISTRATION_RETRY: Duration = Duration::from_secs(60);

// The registry drops a record that is not refreshed within its TTL, so the
// refresh must come first.
const _: () = assert!(HEARTBEAT_INTERVAL_SECS < DEFAULT_REGISTRY_TTL_SECS);

/// The DID the coordinator registers under: `did:key` of its Iroh key.
pub(crate) fn coordinator_did(secret_key: &SecretKey) -> String {
    substrate::derive_did_key(&Identity::from_bytes(&secret_key.to_bytes()).public_key())
}

/// What one registration needs. Shared by every round of the loop.
#[derive(Debug)]
struct Registration {
    registry: RegistryClient,
    secret_key: SecretKey,
    relay_url: Option<String>,
    retry_policy: RetryPolicy,
}

/// Registers the coordinator's record and keeps it registered until the task
/// is aborted. `registered` holds the result of the last attempt.
pub(crate) fn spawn(
    registry_url: String,
    secret_key: SecretKey,
    relay_url: Option<String>,
    retry_policy: RetryPolicy,
    registered: watch::Sender<bool>,
) -> JoinHandle<()> {
    let registration = Arc::new(Registration {
        // The coordinator has never published to the DHT.
        registry: RegistryClient::new(false, Some(registry_url)),
        secret_key,
        relay_url,
        retry_policy,
    });
    tokio::spawn(keep_registered(
        move || {
            let registration = Arc::clone(&registration);
            async move { registration.register_with_retries().await }
        },
        registered,
    ))
}

/// Calls `register` now, then again after `REGISTRATION_REFRESH` when it
/// succeeded or `REGISTRATION_RETRY` when it failed. Never returns.
pub(crate) async fn keep_registered<F, Fut>(mut register: F, registered: watch::Sender<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    loop {
        let ok = register().await;
        registered.send_replace(ok);
        time::sleep(if ok { REGISTRATION_REFRESH } else { REGISTRATION_RETRY }).await;
    }
}

impl Registration {
    async fn register_with_retries(&self) -> bool {
        let res = retry::retry_with_backoff(&self.retry_policy, || self.register_once()).await;
        match &res {
            Ok(()) => debug!("coordinator registered in its community registry"),
            Err(e) => warn!("coordinator registration failed after all retries: {e}"),
        }
        res.is_ok()
    }

    /// Signs a fresh record (new timestamp, so the registry accepts it as
    /// newer) and posts it.
    async fn register_once(&self) -> Result<()> {
        let node_id = self.secret_key.public();
        let endpoint_addr_bytes = serde_json::to_vec(&EndpointAddr::new(node_id))?;
        let identity = Identity::from_bytes(&self.secret_key.to_bytes());
        let did = coordinator_did(&self.secret_key);
        let not_after = /* unchanged: now + DEFAULT_ENDPOINT_NOT_AFTER_SECS */;
        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did,
            endpoint_type: EndpointType::Substrate,
            nickname: Some(format!("coordinator-{}", &node_id.to_string()[..8])),
            mechanisms: vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes,
                relay_url: self.relay_url.clone(),
            }],
            is_private: false,
            ttl: None,
            not_after,
            generation: 0,
        };
        self.registry.register(&info.sign(&identity)?, false).await
    }
}
```

Notes:

- Keep the record fields exactly as today (`coordinator.rs:465-475`). Only
  the transport changes: `RegistryClient::register`
  (`crates/core/src/dht_registry/client.rs`) posts the same JSON to
  `{url}/register` with the shared 10 s timeout, and returns an error for a
  non-`2xx` answer. With the DHT off and `is_private = false` it publishes
  nothing else.
- `register_once` derives the DID with `coordinator_did`, and `start_endpoint`
  calls the same function (step 6), so the DID is derived in one place.
- `keep_registered` takes `F: FnMut() -> Fut`, not an async closure
  (`impl AsyncFnMut() -> bool`). It runs inside `tokio::spawn`, which needs a
  `Send` future. With an async closure that borrows its captures, rustc often
  cannot prove that the returned future is `Send` for every lifetime
  ("implementation of `Send` is not general enough"). Here each call clones
  an `Arc<Registration>` into an `async move` block, so the future owns what
  it uses and is `Send`. `RegistryClient` and `SecretKey` are `Send + Sync`.
- The `const _: () = assert!(…)` line fails the build if someone makes the
  refresh as long as the registry TTL. It needs no test. Do not write it as a
  runtime `assert!` in a test: on two constants that triggers
  `clippy::assertions_on_constants`, and the gate runs clippy with
  `-D warnings`.
- Check the import rule for `retry::retry_with_backoff` against how
  `coordinator.rs` imports it today (it imports the function directly at
  l.28). Follow `AGENTS.md`: functions are called through their module.

Add `#[cfg(test)] mod tests;` at the end of the file.

### Step 6 — `crates/coordinator_iroh/src/coordinator.rs`

`init` is 97 lines today (l.88-184), and the old `spawn_http_info_server`
would need 8 arguments (clippy `too_many_arguments` allows 7). So build the
info state and the router in one new function, and pass a ready router to the
server.

**6a. Helper (free function near `resolve_relay_url`):**

```rust
/// The address `/v1/info` gives to clients. (Move the existing comment from
/// `spawn_http_info_server` l.271-274 here.)
fn info_addr_bytes(node_id: EndpointId, relay_url: Option<&RelayUrl>) -> Result<Vec<u8>> {
    let addr = EndpointAddr::from_parts(node_id, relay_url.cloned().map(TransportAddr::Relay));
    Ok(serde_json::to_vec(&addr)?)
}
```

`EndpointId` is `iroh::EndpointId` (add it to the `iroh::{...}` import).

**6b. Result struct and `start_endpoint`:**

```rust
/// What `start_endpoint` starts. `init` stores each part in `CoordinatorIroh`.
struct EndpointParts {
    iroh_router: IrohRouter,
    http_info_handle: JoinHandle<Result<()>>,
    info_addr: SocketAddr,
    registry_registration_handle: Option<JoinHandle<()>>,
}

impl CoordinatorIroh {
    async fn start_endpoint(
        config: &SubstrateConfig,
        iroh_cfg: &CoordinatorIrohConfig,
        parent_relay_url: Option<&RelayUrl>,
        relay_server: &Option<Server>,
    ) -> Result<EndpointParts> {
        let relay_url = resolve_relay_url(parent_relay_url, relay_server, iroh_cfg)?;
        let (endpoint, secret_key) = Self::build_iroh_endpoint(iroh_cfg, relay_url.clone()).await?;
        let (iroh_router, active_connections) = Self::spawn_iroh_router(
            &endpoint, iroh_cfg, config, parent_relay_url.map(ToString::to_string));

        let registry_url = iroh_cfg
            .community_registry_url
            .clone()
            .filter(|_| iroh_cfg.share_in_registry);
        let (registered_tx, registered_rx) = watch::channel(false);

        let state = InfoState {
            endpoint_addr_bytes: info_addr_bytes(endpoint.id(), relay_url.as_ref())?,
            substrate_id: endpoint.id().to_string(),
            relay_url: relay_url.as_ref().map(ToString::to_string),
            parent_coordinator_url: parent_relay_url.map(ToString::to_string),
            active_connections,
            max_connections: iroh_cfg.max_connections,
            tls_cert_path: config.tls.as_ref().map(|t| t.cert_path.clone()),
            relay_metrics: relay_server.as_ref().map(|s| s.metrics().server.clone()),
            max_relay_clients: iroh_cfg.max_relay_clients,
            registry: registry_url
                .is_some()
                .then(|| (registration::coordinator_did(&secret_key), registered_rx)),
        };
        let (http_info_handle, info_addr) =
            Self::spawn_http_info_server(config, iroh_cfg, info_endpoint::info_router(state)).await?;

        let registry_registration_handle = registry_url.map(|url| {
            registration::spawn(
                url,
                secret_key,
                relay_url.as_ref().map(ToString::to_string),
                config.retry.clone(),
                registered_tx,
            )
        });

        Ok(EndpointParts { iroh_router, http_info_handle, info_addr, registry_registration_handle })
    }
}
```

The order of side effects stays as today: endpoint, router, info server,
registration. These must not change: `relay_url` (the `RelayUrl` string form
with a trailing slash, change doc D3), `substrate_id`, `endpoint_addr_bytes`,
`parent_coordinator_url`, and the registered record's `relay_url` (today
`relay_url.as_ref().map(ToString::to_string)`, l.162).

**6c. `init` (l.88-184)** keeps the access check, the architecture comment
and the relay spawn. After `relay_access(&role.access)?;` add the cap check
from step 2. Replace the body of `if let Some(iroh_cfg) = &role.iroh`
(l.127-174) with:

```rust
let parts = Self::start_endpoint(config, iroh_cfg, parent_relay_url.as_ref(), &relay_server).await?;
iroh_router = Some(parts.iroh_router);
http_info_handle = Some(parts.http_info_handle);
info_addr = Some(parts.info_addr);
registry_registration_handle = parts.registry_registration_handle;
```

**6d. `spawn_http_info_server`** takes the router:

```rust
async fn spawn_http_info_server(
    config: &SubstrateConfig,
    iroh_cfg: &CoordinatorIrohConfig,
    app: Router,
) -> Result<(JoinHandle<Result<()>>, SocketAddr)>
```

Keep l.255-268 (bind address, listener, log) and l.299-324 (TLS branch and
serve). Delete l.270-297 (the state build and the router build, now in
`start_endpoint` and `info_router`). B2 merges its own routes (with their own
state) into `app` before this call, so this function does not change again.

**6e. `registry_registration_handle`.** Add a doc comment on the field:
"Keeps the coordinator registered in its community registry." `shutdown`
already aborts this handle (l.428-430). No change there.

**6f. Imports.** Add `tokio::sync::watch`, `iroh::EndpointId`, and the module
paths `crate::{info_endpoint, registration}`. Remove what moved to
`registration.rs` (`Identity`, `derive_did_key`, `EndpointInfo`,
`EndpointMechanism`, `EndpointType`, `DEFAULT_ENDPOINT_NOT_AFTER_SECS`,
`RegistryClient` if unused, `retry_with_backoff`, `SystemTime`, `UNIX_EPOCH`,
`RetryPolicy`, `reqwest::Client` if only `wait_for_relay_server` still uses
it — it does, so keep that one). `RegistryClient` is still used by
`spawn_iroh_router` (l.229); keep it. Run the import clean-up pass from
`AGENTS.md`.

### Step 7 — unit tests

**7a. `crates/core/src/coordinator_info/tests.rs`:**

- `status_values_keep_their_wire_strings`: `serde_json::to_string` of
  `Healthy` is `"\"healthy\""` and of `AtCapacity` is `"\"at_capacity\""`.
- `unknown_status_string_parses_as_unknown`: `"degraded"` gives
  `CoordinatorStatus::Unknown`.
- `older_answer_without_new_fields_parses`: parse a JSON object with only
  `endpoint_addr_bytes`, `substrate_id`, `relay_url`,
  `parent_coordinator_url`, `"identity": {"did": "x", "controller_status":
  "unverified"}` and `"relay": {"online": true}`. Expect `status == Unknown`,
  `relay.clients == None`, `relay.max_clients == None`, `registry == None`.

**7b. `crates/coordinator_iroh/src/info_endpoint/tests.rs`:**

- `client_count_is_accepts_minus_disconnects`: `RelayServerMetrics::default()`;
  call `accepts.inc()` 3 times and `disconnects.inc()` once; expect 2. Then
  `disconnects.inc()` 3 more times; expect 0 (no underflow).
- `status_is_at_capacity_only_at_or_above_the_cap`: table over
  `relay_status`: `(None, None)`, `(Some(5), None)`, `(None, Some(1))`,
  `(Some(1), Some(2))` give `Healthy`; `(Some(2), Some(2))`,
  `(Some(3), Some(2))` give `AtCapacity`.
- `info_reads_state_without_network`: build an `InfoState` by hand (no TLS
  path, `relay_metrics` with two accepts, `max_relay_clients: Some(2)`,
  `registry: Some(("did:key:test".into(), rx))` with
  `watch::channel(true)`). Call `get_info(State(Arc::new(state))).await`.
  Expect `status == AtCapacity`, `relay.clients == Some(2)`, `relay.online`,
  `identity.is_none()`, `registry.did == "did:key:test"`,
  `registry.registered`.

**7c. `crates/coordinator_iroh/src/registration/tests.rs`:**

- `registration_repeats_on_its_own_timer` with
  `#[tokio::test(start_paused = true)]`: a `register` closure that counts
  calls in an `Arc<AtomicUsize>` and answers `false` on call 1 and `true`
  after. Write it in the same shape as production:
  `move || { let calls = Arc::clone(&calls); async move { calls.fetch_add(1, Ordering::SeqCst) > 0 } }`.
  Spawn `keep_registered(register, tx)` with `tokio::spawn`. Then:
  1. `tokio::task::yield_now()` until the count is 1; expect `*rx.borrow()`
     is `false`.
  2. `time::advance(REGISTRATION_RETRY - 1s)`, yield: count still 1.
  3. `time::advance(1s)`, yield: count 2, `*rx.borrow()` is `true`.
  4. `time::advance(REGISTRATION_REFRESH - 1s)`, yield: count 2.
  5. `time::advance(1s)`, yield: count 3.
  Abort the task at the end.

The rule "refresh before the registry TTL" is a compile-time assert in
`registration.rs` (step 5), not a test.

`start_paused` needs tokio's `test-util` feature, which the workspace `tokio`
entry does not turn on. Add to `crates/coordinator_iroh/Cargo.toml`
`[dev-dependencies]`: `tokio = { workspace = true, features = ["test-util"] }`
(the same pattern as `crates/rpc/Cargo.toml` l.25).

### Step 8 — integration tests

All listeners bind `127.0.0.1:0` (no port literals). Turn the DHT off
(`config.substrate.enable_bep0044_dht = false`). Set
`info_http_bind_address: Some("127.0.0.1:0".into())`. Bound every wait with a
deadline and poll every 50-100 ms. Use the crate-level allow line that the
other test files use.

**Shared helper.** Move `spawn_global_registry` from `multi_hop_relay.rs`
(l.274-291) into `tests/common/mod.rs` as
`pub async fn spawn_registry(base_path: &Path) -> Result<(EcosystemRegistry, String)>`,
and call it from both files. `multi_hop_relay.rs` is in
`xtask/oversized-test-files.txt` (cap 1128). Its line count changes with this
move and the import change. Set its entry to the new real count (`wc -l`),
because the ratchet fails in both directions.

**New file `crates/coordinator_iroh/tests/info_endpoint.rs`.** Helpers:

```rust
async fn fetch_info(coord: &CoordinatorIroh) -> Result<CoordinatorInfo> {
    let addr = coord.info_addr().context("no info address")?;
    Ok(Client::new().get(format!("http://{addr}{INFO_PATH}")).send().await?.json().await?)
}

/// Polls `/v1/info` until `pred` holds, or fails after `deadline` with the
/// last answer in the error.
async fn wait_for_info(coord: &CoordinatorIroh, deadline: Duration,
    pred: impl Fn(&CoordinatorInfo) -> bool) -> Result<CoordinatorInfo>
```

**8a. `relay_count_follows_connections_and_capacity`** (AC-6 server side,
AC-17 count):

1. Start a coordinator: `enable_relay = true`, `http_bind_address` and
   `quic_bind_address` `127.0.0.1:0`, `access = "everyone"` (the default),
   `max_relay_clients = Some(2)`, no registry.
   `common::wait_for_relay(&coord, 20s)`.
2. Wait until `relay.clients == Some(1)` and `status == Healthy`. The one
   connection is the coordinator's own endpoint (D-A3-10).
3. Bind a client endpoint on the coordinator's relay:
   ```rust
   let relay_url: RelayUrl = info.relay_url.unwrap().parse()?;
   let client = Endpoint::empty_builder()
       .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)))
       .bind().await?;
   time::timeout(Duration::from_secs(20), client.online()).await?;
   ```
4. Wait until `relay.clients == Some(2)`, `status == AtCapacity` and
   `relay.max_clients == Some(2)`.
5. `client.close().await`. Wait until `relay.clients == Some(1)` and
   `status == Healthy`.
6. `coord.shutdown().await`.

If step 2 never reaches 1, first find out whether the coordinator's endpoint
connects to its own relay (log `relay.clients`). Do not loosen the test to
`>= 1` without knowing why.

**8b. `coordinator_record_is_registered_under_its_did`:**

1. `common::spawn_registry(tmp)`.
2. Start a coordinator with `enable_relay = true`,
   `community_registry_url = Some(r_url)`, `share_in_registry = true`.
3. Wait (20 s deadline) until `registry.registered`.
4. `RegistryClient::new(false, Some(r_url)).lookup(&registry.did, false)`
   succeeds, and its `info.service_id == registry.did`. `registry.did` starts
   with `did:key:` and differs from `substrate_id`. `identity` is `None`.

**8c. `probes_do_not_call_the_registry`** (AC-17):

1. Start a mock registry: an `axum::Router` on `127.0.0.1:0` with
   `POST /register` → add 1 to a register counter, answer `200`; and
   `GET /lookup/{id}` → add 1 to a lookup counter, answer `404`. (Use the same
   axum path syntax as `crates/community_registry/src/registry.rs:135`.)
2. Start a coordinator with `enable_relay = false`,
   `community_registry_url = Some(mock_url)`, `share_in_registry = true`.
3. Wait until `registry.registered` (10 s deadline).
4. Send 20 `GET /v1/info` requests. Each must answer within 2 s
   (`time::timeout`).
5. Expect the lookup counter to be 0 and the register counter to be 1 (the
   next registration is an hour away).
6. On the last answer: `relay.online == false`, `relay.clients == None`,
   `status == Healthy` (no relay server means no capacity check, D-A3-8).

**8d. `connection_limit.rs`: the router cap does not give `at_capacity`.**
In `accepts_up_to_cap_and_rejects_the_rest`, after `tally_verdicts` and
before `release.notify_waiters()` (l.103-107), while the accepted connections
are still held: poll `/v1/info` (deadline `SETTLE_DEADLINE`) until
`connections.active == CAP`, then assert `status == Healthy` (D-A3-8). Keep
the file's existing `fetch_dial_info` style for the request.

**8e. `startup_config.rs`: `rejects_zero_max_relay_clients`.** `test_config()`
with `enable_relay = true` and `max_relay_clients = Some(0)`. `init` fails,
and the error contains `max_relay_clients`.

## 4. Things that are ambiguous or stale in task.md and the change docs

For A3:

1. **The registry lookup in `/v1/info` never matched**, so
   `controller_status` was always `unverified`, and `identity` had a meaning
   different from the M02 schema. Not mentioned in task.md or the change doc.
   See D-A3-4 to D-A3-6.
2. **No config key for the relay client cap.** AC-6 says how to count, not
   what to compare with. See D-A3-1.
3. **The cap cannot be enforced with the `iroh-relay` 0.97 API.** See
   D-A3-2.
4. **AC-17 "cached" lookup.** The plan removes the lookup instead (D-A3-6).
   This meets the criterion and is stronger.
5. **AC-6 counts "relay clients".** The counters count connections, not
   endpoints (D-A3-9). The difference is small and short, and AC-6's rule is
   kept as written.
6. **Where the wire type lives.** Neither task.md nor the change doc says that
   A4 needs `CoordinatorInfo` from a crate the substrate always links. See
   D-A3-12.
7. **AC-6 is mostly substrate side.** A3 does only the server half. Leave
   AC-6's Test column with the server tests and the note "substrate skip in
   A4", so A4's close-out does not skip it.
8. **task.md §3 says A3 runs "Parallel with A2".** A2 is merged now (#302),
   so this no longer applies. A3 is based on `main` (§0).

Stale facts in task.md that later slices rely on (fix them in the same docs
commit, completion step 4):

- §1: "The info server state already holds the host `substrate_id`
  (`crates/coordinator_iroh/src/info_endpoint.rs`), which is the passport
  root." Wrong. `InfoState.substrate_id` is the coordinator's own Iroh
  endpoint id, from a key made at random at every start
  (`build_iroh_endpoint`). It is not the host substrate's DID and is not
  stable. The operator-passport change doc already says this ("A coordinator
  role has no stable DID of its own"). Rewrite: B2 must pass the host
  substrate's node DID and controller into the info server, and report them
  in `identity` (M02 meaning, D-A3-4).
- §3 B2: "*Unverified:* the test harness can run the info server with TLS."
  It can: `crates/coordinator_iroh/tests/tls_rotation.rs` runs the info
  server with `[tls]` and calls `https://{info_addr}/v1/info` (l.109).
  Rewrite as verified, with that file as the evidence.
- The operator-passport change doc cites `coordinator.rs:193-200` for the
  random key. It is in `build_iroh_endpoint` (l.203 today). Cite the function
  name, not the line.

## 5. Completion steps

The implementer must follow these steps.

1. **Verify.** Run `mise run verify` as a background job (about 20 minutes)
   and read its summary. Every gate must pass. No `--skip` in the final run.
   While you iterate:
   `cargo nextest run -p syneroym-core coordinator_info`,
   `cargo nextest run -p syneroym-coordinator-iroh`,
   `cargo build -p syneroym-smoke-tests`.
   Also run `cargo dupes report` filtered to the changed files (recipe in
   `AGENTS.md`) and lower `MAX_EXACT_DUPLICATE_PERCENT` if the gate says so.
2. **Change doc** `docs/planning/changes/registry-listed-coordinators/change.md`:
   - AC-6 Test column: "server side:
     `crates/coordinator_iroh/tests/info_endpoint.rs`
     (`relay_count_follows_connections_and_capacity`),
     `crates/coordinator_iroh/tests/connection_limit.rs`,
     `crates/coordinator_iroh/src/info_endpoint/tests.rs`; substrate skip in
     A4".
   - AC-17 Test column: `crates/coordinator_iroh/tests/info_endpoint.rs`
     (`relay_count_follows_connections_and_capacity`,
     `probes_do_not_call_the_registry`).
   - Tick the task "Coordinator info endpoint: relay client count for
     `at_capacity`, cache the registry lookup."
   - Add two lines next to the existing "Note for Slice A4" lines:
     - "Note for Slice A4: use an entry only when `/v1/info` answers with
       `status == "healthy"` and `relay.online == true`. Skip it when
       `status` is `at_capacity` or `Unknown` (a value this build does not
       know), and when `relay` is missing or `relay.online` is false: a
       coordinator with no relay server reports `healthy`."
     - "Note for Slice A4: read `/v1/info` with
       `syneroym_core::coordinator_info::{CoordinatorInfo, INFO_PATH}`;
       the URL is `info_url` (normalised, no trailing slash) followed by
       `INFO_PATH`."
   - `## Deviations`: D-A3-1 to D-A3-12, one line each, plus any change you
     made to this plan.
   - Add to `living-docs-touched` every living doc you change (it already
     lists `docs/system-architecture.md`). Keep `status: in-progress`.
3. **Living docs.**
   - `docs/system-architecture.md`:
     - the paragraph that starts "`GET /v1/info` on a coordinator returns"
       (~l.379): `status` is `at_capacity` when the relay has at least
       `max_relay_clients` open connections, `healthy` otherwise, and
       `healthy` for a coordinator with no relay server; `relay.clients` is
       the number of open relay connections (an endpoint that connects again
       counts twice until its old connection closes); it includes the
       coordinator's own endpoint only when that endpoint uses this relay and
       the relay admits it (with `access = "everyone"`; a list never contains
       the coordinator's random id); the cap is reported, not enforced;
       `connections` is the Syneroym connection count and its cap
       `max_connections`; `registry` (only when the coordinator registers
       itself) gives the DID it registers under and whether the last
       registration was accepted; `identity` is not reported yet.
     - config table (~l.355-374): add
       `[roles.coordinator.iroh] max_relay_clients` | None (no cap) | "Relay
       connections at which `/v1/info` reports `at_capacity`. Reported only;
       the relay still admits more clients. `0` stops startup."
     - config table row `community_registry_url`, `share_in_registry`
       (l.369): replace "registers itself in that registry once at startup"
       with "registers itself in that registry at startup, and again every
       hour (every minute after a failure), so its record does not expire."
     - Multi-Hop section, the Cp bullet (l.1763): replace "Cp registers once
       at startup, with retries if the call fails. It does not register
       again. … So this record is gone from the registry about 2 hours after
       Cp starts." with: Cp registers at startup, with retries if the call
       fails, and registers again every hour (every minute after a failure).
       The registry deletes an entry that is not refreshed within 2 hours, so
       the hourly refresh keeps Cp's record.
     - Registry section (l.2106): replace "registers itself once at startup,
       with retries. It has no republish loop." with: registers itself at
       startup, with retries, and again every hour (every minute after a
       failure). `/v1/info` shows the DID it registers under and whether the
       last registration was accepted (`registry`).
     - l.2523 (the coordinator uses the retry policy when it registers) stays
       true. Do not change it.
   - `docs/system-requirements-spec.md`, `docs/developer-guide.md`,
     `docs/TERMINOLOGY.md`, `docs/planning/traceability-matrix.md`: no change
     expected. Check once (`git grep -n -e 'v1/info' -e 'controller_status'`).
4. **Status, task and backlog.**
   - `status.md`: split the last row "A3 to A4, B1 to B3, C1 to C8 | Not
     started" into an A3 row (Done, the tool that implemented it, the
     `mise run verify` result and the test files) and a row "A4, B1 to B3, C1
     to C8 | Not started".
   - `task.md`: fix the stale facts listed in §4. Under §3 B2, add two
     bullets:
     - "`/v1/info` `identity` is `null` since A3. Fill `identity.did` with the
       host substrate's node DID and `identity.controller_status` with the
       controller check (the M02 meaning). These are also the passport root."
     - "Decide whether the coordinator's own endpoint is always admitted on
       its own relay. With `access = "passport"` it holds no lease, so it
       would be refused and have no home relay. `spawn_http_info_server`
       takes a ready `axum::Router`; merge the passport routes into it."
   - `docs/planning/deferred-backlog.md`:
     - new row "The relay client cap is reported, not enforced" (D-A3-2:
       `iroh-relay` 0.97 has no client limit; the access closure could
       refuse, but it runs before a reconnect replaces the old connection,
       and B2 rewrites it). Target `TBD`. Link
       `crates/coordinator_iroh/src/info_endpoint.rs`.
     - the row "A coordinator registers itself once, so its registry record
       expires after 2 hours" (l.274): resolved by D-A3-6. Move it to
       "Recently resolved" with one line and the new file
       `crates/coordinator_iroh/src/registration.rs`.
     - a row for any `TODO`/`FIXME` you leave.
5. **Stop.** Do not start A4 or B2. If you are not making progress after a
   reasonable number of attempts, stop and report what blocks you, with the
   failing output.
