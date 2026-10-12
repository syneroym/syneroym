# M06D slice A3 — Coordinator info: implementation plan

*Reader: the developer (or AI) who implements slice A3. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  (no "M06D", "A3", "X7", "AC-6", "AC-17" in code, comments or test names; ADR
  ids are fine), simple English in comments and docs, the 800-line file cap,
  the 100-line function cap, and the module layout rule (`foo.rs` beside
  `foo/`, never `mod.rs` under `src/`).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 (row X7), §3 (slices A3, A4 and B2). B2 adds passport routes to the same
  info server later; do not build them.
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-6,
  AC-17, the Tasks list and `## Deviations`.
- This plan, §2 (the open decisions). **Confirm the four open decisions with
  the user before you write code.** The plan is written with the recommended
  answer for each one.

**Branch.** A3 does not need A2's code, but A2 (`feat/m06d-a2-implementation`)
is not merged into `main` yet, and both slices edit the same docs
(`docs/system-architecture.md` config table, the change doc's Tasks and
`## Deviations`, `status.md`, `docs/planning/deferred-backlog.md`).

- If A2 is merged when you start: branch from `origin/main`.
  ```bash
  git fetch origin main
  git checkout -b feat/m06d-a3-coordinator-info origin/main
  ```
- If A2 is not merged: branch from the A2 branch, so the doc edits do not
  conflict. After A2 is squash-merged, move A3 onto `main` with
  `git rebase --onto origin/main <last A2 commit>` (today the last A2 commit
  is `59ab5c9f`).

Push with `git push -u origin feat/m06d-a3-coordinator-info`. Do not open a
pull request unless the user asks. Commit messages follow the Conventional
Commits and 50/72 rules in `AGENTS.md`.

### Terms used in this plan

- **Info server.** The small HTTP (or HTTPS, when `[tls]` is set) server of an
  Iroh coordinator that serves `GET /v1/info`. It has its own bind address
  (`info_http_bind_address`, or the relay port plus 10). It is built in
  `crates/coordinator_iroh/src/coordinator.rs` (`spawn_http_info_server`).
- **Relay client.** An Iroh endpoint that holds an open connection to the
  relay server of this coordinator. The coordinator's own endpoint is one,
  when it uses its own relay as its home relay.
- **Router connection.** A Syneroym connection (ALPN `SYNEROYM_ALPN`) to the
  coordinator's own endpoint. `max_connections` caps these. They are not relay
  clients.
- **Controller status.** The `identity.controller_status` field of
  `/v1/info`. `verified` means the community registry returns this
  coordinator's own endpoint record.

## 1. What this slice delivers

1. `crates/core`: a new module `coordinator_info` with the `/v1/info` wire
   types, moved from `crates/coordinator_iroh`, with typed `status` and
   `controller_status` enums and the new relay client fields. A4 (in
   `crates/substrate`) reads this answer, and `crates/substrate` reaches
   `crates/coordinator_iroh` only behind the `coordinator_iroh` feature. So
   the wire types must live in `crates/core`.
2. `crates/core` config: `[roles.coordinator.iroh] max_relay_clients`.
3. `crates/coordinator_iroh`:
   - `/v1/info` reports the relay client count (`relay.clients`), computed
     as `accepts - disconnects` of the `iroh-relay` server counters.
   - `status` is `at_capacity` when `relay.clients >= max_relay_clients`.
     Router connections no longer decide `status` (AC-6).
   - The handler does no network call. The registry lookup for
     `controller_status` runs in the background, on a timer, after the
     coordinator registers itself. Probes only read the last result (AC-17).
   - The lookup uses the coordinator's DID, which is the id it registers
     under. Today it uses the Iroh endpoint id, so the lookup never succeeds
     (see §4 item 1).
4. Tests for the server side of AC-6, and for AC-17.
5. Living docs, change doc, backlog and status updates.

Not in this slice: the substrate resolver and its probe of `info_url` (A4),
passport routes and the TLS rule on the info server (B2), enforcing the relay
client cap (see D-A3-2), re-registering the coordinator before its registry
record expires (backlog row exists).

## 2. Decisions

### Open: confirm with the user before you write code

**OQ-1. Which cap decides `at_capacity`?** AC-6 says the count is relay
clients, not router connections, but no config key sets a relay client cap.
`max_connections` already caps router connections (`RouteHandler` refuses
connection number `cap + 1` with `ServiceUnavailable`).
*Recommended:* a new key `[roles.coordinator.iroh] max_relay_clients:
Option<u64>`, default `None` (no cap, `status` is always `healthy`). Reusing
`max_connections` would give one number two meanings and would change the
router cap of every coordinator that sets it.

**OQ-2. Is the relay client cap enforced, or only reported?** `iroh-relay`
0.97 has no client limit (`Limits` has only accept rate and per-client byte
rate). The relay `AccessConfig::Restricted` closure could refuse a new client
at the cap, but it runs before the connection replaces an older connection of
the same endpoint, so a reconnecting client could be refused by its own old
connection. B2 also rewrites this closure for passports.
*Recommended:* report only. A substrate skips an `at_capacity` relay (A4).
Add a backlog row for enforcement (completion step 4).

**OQ-3. Move the `/v1/info` wire types to `crates/core`?**
*Recommended:* yes, in this slice (reason in §1 item 1). The old path
`syneroym_coordinator_iroh::CoordinatorInfo` goes away; three call sites
change (step 4). Doing it in A4 instead means A4 edits this module again.

**OQ-4. Fix the lookup id and `identity.did`?** Today the handler looks up
`state.substrate_id`, which is the coordinator's Iroh endpoint id
(`coordinator.rs:284`). The coordinator registers under its DID
(`derive_did_key`, `coordinator.rs:457`). The registry keys records by DID
and by a `nickname-shorthash` alias (`registry.rs:209-225,311`), so the
lookup never matches, and `controller_status` is always `unverified`.
`identity.did` also shows the endpoint id, not a DID.
*Recommended:* yes. Caching a lookup that can never succeed has no value.
`substrate_id` keeps the endpoint id, because clients dial with it
(`connection_limit.rs:213`, `multi_hop_relay.rs`).

### Made in this plan

- **D-A3-1. Config key name:** `max_relay_clients` (OQ-1).
- **D-A3-2. The cap is reported, not enforced** (OQ-2).
- **D-A3-3. `status` reads relay clients only.** A coordinator with no relay
  server, or with no `max_relay_clients`, reports `healthy`. Reaching
  `max_connections` no longer gives `at_capacity`. `connections.active` and
  `connections.cap` stay in the answer for operators.
- **D-A3-4. No lookup in the handler.** AC-17 says "the registry lookup inside
  it is cached". The plan goes one step further: the handler never does a
  lookup. A background task does it on a timer, so the number of lookups does
  not depend on the number of probes at all, and a slow registry cannot slow a
  probe (A4 probes with a timeout).
- **D-A3-5. The lookup runs only after the coordinator registered itself.**
  The task is the existing registration task: register (with the existing
  retry), then check the own record every `RECORD_CHECK_UNVERIFIED` (60 s)
  while it is not found, and every `RECORD_CHECK_VERIFIED` (10 min) while it
  is found. When registration fails after all retries, the task ends and the
  status stays `unverified`. A coordinator without `share_in_registry` and
  `community_registry_url` never registers, so it never looks up; it reports
  `unverified` as today.
- **D-A3-6. The lookup asks the HTTP registry only, never the DHT.** The
  coordinator posts its record to the registry only
  (`register_coordinator_in_registry`); it never publishes to the DHT. A DHT
  lookup can never find it and costs a slow DHT query. So the lookup client is
  `RegistryClient::new(false, Some(registry_url))`.
- **D-A3-7. Wire changes.** All field names stay. `status` becomes an enum
  that serialises to the same strings (`healthy`, `at_capacity`).
  `controller_status` becomes an enum (`verified`, `unverified`). Both enums
  read an unknown string as `Unknown` (`#[serde(other)]`), so an older or
  newer peer still parses. `relay` gains `clients` and `max_clients`.
  `identity.did` becomes the DID (OQ-4).
- **D-A3-8. The relay client count includes the coordinator's own endpoint**
  when that endpoint uses this relay (no parent relay). It is a real client of
  the relay, so it is not subtracted. Write this in the architecture doc.
- **D-A3-9. `relay.online` keeps its meaning** ("this coordinator runs a relay
  server"), now read as "the relay counters exist". The field name is not
  changed.

## 3. Step-by-step changes

Files touched:

```text
crates/core/src/lib.rs                               add `pub mod coordinator_info;`
crates/core/src/coordinator_info.rs                  new: wire types (step 1)
crates/core/src/coordinator_info/tests.rs            new: serde tests (step 6)
crates/core/src/config/roles.rs                      new key (step 2)
crates/coordinator_iroh/Cargo.toml                   dev-dep tokio test-util (step 6)
crates/coordinator_iroh/src/lib.rs                   module visibility, re-export (step 4)
crates/coordinator_iroh/src/info_endpoint.rs         handler, count, record check (step 3)
crates/coordinator_iroh/src/info_endpoint/tests.rs   new: unit tests (step 6)
crates/coordinator_iroh/src/coordinator.rs           wiring and refactor (step 5)
crates/coordinator_iroh/tests/common/mod.rs          shared registry helper (step 7)
crates/coordinator_iroh/tests/info_endpoint.rs       new: integration tests (step 7)
crates/coordinator_iroh/tests/connection_limit.rs    import path (step 4)
crates/coordinator_iroh/tests/multi_hop_relay.rs     import path, helper move (steps 4, 7)
crates/smoke-tests/src/main.rs                       import path, `{:?}` (step 4)
xtask/oversized-test-files.txt                       multi_hop_relay.rs count (step 7)
```

### Step 1 — `crates/core/src/coordinator_info.rs` (new)

Add `pub mod coordinator_info;` to `crates/core/src/lib.rs` (keep the list
sorted: after `config`, before `curated_list`).

Move the five structs out of `crates/coordinator_iroh/src/info_endpoint.rs`
(lines 19-63) into this file. Change them as below. Keep the field order and
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
    /// The relay has at least `max_relay_clients` clients. A substrate picks
    /// another relay.
    AtCapacity,
    /// A value this build does not know, or no value.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Whether the community registry returns this coordinator's own record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerStatus {
    Verified,
    Unverified,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityInfo {
    /// The DID the coordinator registers under (`did:key` of its Iroh key).
    pub did: String,
    pub controller_status: ControllerStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionsInfo {
    /// Open Syneroym connections to the coordinator's endpoint. Not relay
    /// clients.
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
    /// Clients connected to the relay now. `None` when there is no relay
    /// server.
    #[serde(default)]
    pub clients: Option<u64>,
    /// `max_relay_clients`.
    #[serde(default)]
    pub max_clients: Option<u64>,
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
}
```

Check: `#[serde(other)]` and `#[default]` on the same unit variant compile
together (serde 1.0 and Rust 2024 both allow it). If clippy or serde rejects
it, write `impl Default` by hand.

Add `#[cfg(test)] mod tests;` at the end of the file (tests in step 6).

### Step 2 — `crates/core/src/config/roles.rs`: the cap

In `CoordinatorIrohConfig` (l.297), add after `max_connections`:

```rust
    /// Relay clients at which `/v1/info` reports `at_capacity`. Reported
    /// only: the relay still admits more clients. `None` means no cap.
    pub max_relay_clients: Option<u64>,
```

In `impl Default for CoordinatorIrohConfig` (l.309) add
`max_relay_clients: None,`.

Every struct literal of `CoordinatorIrohConfig` in the workspace ends with
`..Default::default()` (checked: 13 files), so no other call site changes.

If the repo has a commented config template that lists
`[roles.coordinator.iroh]` keys, add the key there too. (A2 found no such
template for this section; check with
`git grep -n 'info_http_bind_address' -- '*.toml'`.)

### Step 3 — `crates/coordinator_iroh/src/info_endpoint.rs`

After step 1 the file keeps: `InfoState`, `get_cert_expiry_days`, `get_info`.
Add: `relay_client_count`, `relay_status`, `watch_own_record`, and the two
interval constants. Make the module crate-private (step 4), so its items are
`pub(crate)`.

Imports (types via `use`, functions through their module, per `AGENTS.md`):

```rust
use std::{fs, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};

use axum::{Json, extract::State};
use chrono::Utc;
use iroh_relay::server::Metrics as RelayServerMetrics;
use syneroym_core::coordinator_info::{
    ConnectionsInfo, ControllerStatus, CoordinatorInfo, CoordinatorStatus, IdentityInfo,
    RelayInfo, TlsInfo,
};
use tokio::{sync::watch, time};
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
    /// The DID the coordinator registers under.
    pub did: String,
    pub relay_url: Option<String>,
    pub parent_coordinator_url: Option<String>,
    pub active_connections: Arc<AtomicUsize>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<PathBuf>,
    /// Counters of the relay server this coordinator runs. `None` when it runs
    /// no relay server.
    pub relay_metrics: Option<Arc<RelayServerMetrics>>,
    pub max_relay_clients: Option<u64>,
    /// Last result of the background record check. The handler only reads it.
    pub controller_status: watch::Receiver<ControllerStatus>,
}
```

Removed fields: `is_relay_enabled` (now `relay_metrics.is_some()`, see D-A3-9;
the relay server exists exactly when `enable_relay` is true, `coordinator.rs:123`)
and `registry_client`.

**3b. Relay client count.** Fact checked in
`iroh-relay-0.97.0/src/server/client.rs:276-294`: each client actor calls
`metrics.accepts.inc()` when it starts and `metrics.disconnects.inc()` when it
ends, always as a pair. A client refused by the access check never starts an
actor (`http_server.rs:750`), so it is not counted. A second connection from
the same endpoint replaces the first, and the first one's actor ends, so the
count is about one per endpoint. `Counter::get` returns 0 without the
`iroh-metrics/metrics` feature; the `iroh-relay` `server` feature (workspace
`Cargo.toml` l.194) turns on `metrics`, so the counters work.

```rust
/// Relay clients connected now. `iroh-relay` has no gauge for this; each
/// client actor counts one accept when it starts and one disconnect when it
/// ends.
pub(crate) fn relay_client_count(metrics: &RelayServerMetrics) -> u64 {
    // Read disconnects first: a client's disconnect is counted after its
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

**3c. Handler.** No network call, no `await` on I/O:

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
        identity: Some(IdentityInfo {
            did: state.did.clone(),
            controller_status: *state.controller_status.borrow(),
        }),
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
    })
}
```

Keep `async fn`: axum needs it as a handler.

**3d. Background record check.**

```rust
/// How often the coordinator checks that the registry returns its record,
/// while the registry returns it.
pub(crate) const RECORD_CHECK_VERIFIED: Duration = Duration::from_secs(600);
/// ... while the registry does not return it.
pub(crate) const RECORD_CHECK_UNVERIFIED: Duration = Duration::from_secs(60);

/// Checks on a timer that the registry returns the coordinator's own record,
/// and publishes the result for `/v1/info`. Runs until its task is aborted.
pub(crate) async fn watch_own_record(
    mut lookup: impl AsyncFnMut() -> bool,
    status: watch::Sender<ControllerStatus>,
) {
    loop {
        let verified = lookup().await;
        let next = if verified {
            status.send_replace(ControllerStatus::Verified);
            RECORD_CHECK_VERIFIED
        } else {
            status.send_replace(ControllerStatus::Unverified);
            RECORD_CHECK_UNVERIFIED
        };
        time::sleep(next).await;
    }
}
```

`send_replace` (not `send`) stores the value even when no receiver is left.
`AsyncFnMut` is stable since Rust 1.85; the workspace uses edition 2024 on
stable. No other crate uses it yet. If it causes trouble, take
`F: FnMut() -> Fut, Fut: Future<Output = bool>` and give the closure an
`Arc<RegistryClient>` clone.

Add `#[cfg(test)] mod tests;` at the end of the file.

### Step 4 — `lib.rs` and the three call sites of the moved type

`crates/coordinator_iroh/src/lib.rs`:

```rust
mod config;
mod coordinator;
mod info_endpoint;

pub use coordinator::CoordinatorIroh;
```

(Remove `pub mod info_endpoint` and `pub use info_endpoint::CoordinatorInfo`.)

Call sites (all checked with `git grep -n CoordinatorInfo`):

| File | Line | Change |
| --- | --- | --- |
| `crates/coordinator_iroh/tests/connection_limit.rs` | 17 | `use syneroym_coordinator_iroh::CoordinatorIroh;` and add `coordinator_info::CoordinatorInfo` to the existing `syneroym_core::{...}` import (l.18) |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs` | 21 | same as above (the `syneroym_core::{...}` import is l.22) |
| `crates/smoke-tests/src/main.rs` | 11 | `use syneroym_coordinator_iroh::CoordinatorIroh;` and import `syneroym_core::coordinator_info::CoordinatorInfo` |
| `crates/smoke-tests/src/main.rs` | 166 | `println!("  Status:       {:?}", info.status);` (the enum has no `Display`) |

No code reads `info.identity` or compares `info.status` as a string (checked).

### Step 5 — `crates/coordinator_iroh/src/coordinator.rs`

`init` is 97 lines today (l.88-184). New lines would push it over the
100-line limit, and `spawn_http_info_server` would need 8 arguments
(clippy `too_many_arguments` allows 7). So extract the per-endpoint work into
one function and build `InfoState` there.

**5a. New helpers (free functions near `resolve_relay_url`):**

```rust
/// The DID the coordinator registers under: `did:key` of its Iroh key.
fn coordinator_did(secret_key: &SecretKey) -> String {
    derive_did_key(&Identity::from_bytes(&secret_key.to_bytes()).public_key())
}

/// The address `/v1/info` gives to clients. (Move the existing comment from
/// `spawn_http_info_server` l.271-274 here.)
fn info_addr_bytes(node_id: EndpointId, relay_url: Option<&RelayUrl>) -> Result<Vec<u8>> {
    let addr = EndpointAddr::from_parts(node_id, relay_url.cloned().map(TransportAddr::Relay));
    Ok(serde_json::to_vec(&addr)?)
}
```

Derive the DID in one place only. `register_coordinator_in_registry`
(l.446) derives it again today (l.456-457). Give it a new argument
`did: &str` (six arguments) and delete its `let did = …` line. It still builds
`identity` from `secret_key_bytes`, because it signs the record. Use `did` where
it used the local `did` (`service_id` and `substrate_id`).

`EndpointId` is `iroh::EndpointId` (add it to the `iroh::{...}` import).

**5b. Result struct and `start_endpoint`:**

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
        let did = coordinator_did(&secret_key);
        let (iroh_router, active_connections) = Self::spawn_iroh_router(
            &endpoint, iroh_cfg, config, parent_relay_url.map(ToString::to_string));

        // Stays `Unverified` when the coordinator never registers.
        let (status_tx, status_rx) = watch::channel(ControllerStatus::Unverified);
        let state = InfoState {
            endpoint_addr_bytes: info_addr_bytes(endpoint.id(), relay_url.as_ref())?,
            substrate_id: endpoint.id().to_string(),
            did: did.clone(),
            relay_url: relay_url.as_ref().map(ToString::to_string),
            parent_coordinator_url: parent_relay_url.map(ToString::to_string),
            active_connections,
            max_connections: iroh_cfg.max_connections,
            tls_cert_path: config.tls.as_ref().map(|t| t.cert_path.clone()),
            relay_metrics: relay_server.as_ref().map(|s| s.metrics().server.clone()),
            max_relay_clients: iroh_cfg.max_relay_clients,
            controller_status: status_rx,
        };
        let (http_info_handle, info_addr) =
            Self::spawn_http_info_server(config, iroh_cfg, state).await?;

        let registry_registration_handle = match &iroh_cfg.community_registry_url {
            Some(registry_url) if iroh_cfg.share_in_registry => Some(Self::register_in_global_registry(
                registry_url.clone(), secret_key, relay_url.as_ref().map(ToString::to_string),
                did, config.retry.clone(), status_tx)),
            _ => None,
        };

        Ok(EndpointParts { iroh_router, http_info_handle, info_addr, registry_registration_handle })
    }
}
```

The order of side effects stays as today: endpoint, router, info server,
registration. `relay_url` in `InfoState` stays the `RelayUrl` string form with
a trailing slash (change doc D3). Behaviour for `relay_url`, `substrate_id`,
`endpoint_addr_bytes` and `parent_coordinator_url` must not change.

**5c. `init` (l.88-184)** keeps the access check, the architecture comment
and the relay spawn. Replace the body of `if let Some(iroh_cfg) = &role.iroh`
(l.127-174) with:

```rust
if iroh_cfg.max_relay_clients.is_some() && !iroh_cfg.enable_relay {
    warn!("[roles.coordinator.iroh] max_relay_clients is set, but enable_relay is false; it has no effect");
}
let parts = Self::start_endpoint(config, iroh_cfg, parent_relay_url.as_ref(), &relay_server).await?;
iroh_router = Some(parts.iroh_router);
http_info_handle = Some(parts.http_info_handle);
info_addr = Some(parts.info_addr);
registry_registration_handle = parts.registry_registration_handle;
```

**5d. `spawn_http_info_server`** takes the state:

```rust
async fn spawn_http_info_server(
    config: &SubstrateConfig,
    iroh_cfg: &CoordinatorIrohConfig,
    state: InfoState,
) -> Result<(JoinHandle<Result<()>>, SocketAddr)>
```

Keep l.255-268 (bind address, listener, log) and l.297-324 (router, TLS
branch). Delete l.270-295 (the `InfoState` build, now in `start_endpoint`).
Route with the core constant:
`Router::new().route(INFO_PATH, get(get_info)).with_state(Arc::new(state))`.

**5e. `register_in_global_registry`** becomes:

```rust
fn register_in_global_registry(
    registry_url: String,
    secret_key: SecretKey,
    relay_url_payload: Option<String>,
    did: String,
    retry_policy: RetryPolicy,
    controller_status: watch::Sender<ControllerStatus>,
) -> JoinHandle<()> {
    let node_id = secret_key.public();
    let node_id_str = node_id.to_string();
    let endpoint_addr_payload = EndpointAddr::new(node_id);
    let secret_key_bytes = secret_key.to_bytes();
    tokio::spawn(async move {
        // Unchanged closure, except that it also clones `did` and passes
        // `&did` to `register_coordinator_in_registry` (step 5a).
        let res = retry_with_backoff(&retry_policy, || { /* … */ }).await;
        if let Err(e) = res {
            warn!("Failed to register coordinator in global registry after all retries: {}", e);
            return;
        }
        info!("Coordinator successfully registered in global registry");

        // The coordinator posts its record to the HTTP registry only, never
        // to the DHT, so a DHT lookup could not find it.
        let registry = RegistryClient::new(false, Some(registry_url));
        info_endpoint::watch_own_record(
            async || registry.lookup(&did, false).await.is_ok(),
            controller_status,
        )
        .await;
    })
}
```

`node_id_str` and `endpoint_addr_payload` were arguments before; they come
from `secret_key` now (same values: `init` passed `node_id.to_string()` and
`EndpointAddr::new(node_id)` with `node_id = endpoint.addr().id`, the public
key of `secret_key`). Six arguments.

Update the doc comment of the `registry_registration_handle` field (or add
one): "Registers the coordinator, then checks on a timer that the registry
returns its record." `shutdown` already aborts this handle (l.428-430), which
also stops the record check. No change there.

**5f. Imports.** Add `tokio::sync::watch`, `iroh::EndpointId`,
`syneroym_core::coordinator_info::{ControllerStatus, INFO_PATH}`. Remove
`std::sync::atomic::AtomicUsize` only if it is no longer used (it is still
used by `spawn_iroh_router`'s return type). Run the import clean-up pass from
`AGENTS.md`.

### Step 6 — unit tests

**6a. `crates/core/src/coordinator_info/tests.rs`:**

- `status_values_keep_their_wire_strings`: `serde_json::to_string` of
  `Healthy` is `"\"healthy\""`, `AtCapacity` is `"\"at_capacity\""`,
  `Verified` is `"\"verified\""`, `Unverified` is `"\"unverified\""`.
- `unknown_status_strings_parse_as_unknown`: `"degraded"` gives
  `CoordinatorStatus::Unknown` and `ControllerStatus::Unknown`.
- `older_answer_without_new_fields_parses`: parse a JSON object with only
  `endpoint_addr_bytes`, `substrate_id`, `relay_url`, `parent_coordinator_url`
  and `"relay": {"online": true}`. Expect `status == Unknown`,
  `relay.clients == None`, `relay.max_clients == None`.

**6b. `crates/coordinator_iroh/src/info_endpoint/tests.rs`:**

- `client_count_is_accepts_minus_disconnects`: `RelayServerMetrics::default()`;
  call `accepts.inc()` 3 times and `disconnects.inc()` once; expect 2. Then
  `disconnects.inc()` 3 more times; expect 0 (no underflow).
- `status_is_at_capacity_only_at_or_above_the_cap`: table over
  `relay_status`: `(None, None)`, `(Some(5), None)`, `(None, Some(0))`,
  `(Some(1), Some(2))` give `Healthy`; `(Some(2), Some(2))`,
  `(Some(3), Some(2))` give `AtCapacity`.
- `info_reads_state_without_network`: build an `InfoState` by hand (no TLS
  path, `relay_metrics: Some(Arc::new(RelayServerMetrics::default()))` with two
  accepts, `max_relay_clients: Some(2)`, a `watch::channel(Verified)`
  receiver, a fixed `did`). Call `get_info(State(Arc::new(state))).await`.
  Expect `status == AtCapacity`, `relay.clients == Some(2)`,
  `relay.online`, `identity.controller_status == Verified`, `identity.did`
  equal to the given DID.
- `record_check_runs_on_its_own_timer` with
  `#[tokio::test(start_paused = true)]`: a lookup closure that counts calls in
  an `Arc<AtomicUsize>` and answers `false` on call 1 and `true` after. Spawn
  `watch_own_record(lookup, tx)`. Then:
  1. `tokio::task::yield_now()` until the count is 1; expect `Unverified`.
  2. `time::advance(RECORD_CHECK_UNVERIFIED - 1s)`, yield: count still 1.
  3. `time::advance(1s)`, yield: count 2, status `Verified`.
  4. `time::advance(RECORD_CHECK_VERIFIED - 1s)`, yield: count 2.
  5. `time::advance(1s)`, yield: count 3.
  Abort the task at the end.

`start_paused` needs tokio's `test-util` feature, which the workspace
`tokio` entry does not turn on. Add to
`crates/coordinator_iroh/Cargo.toml` `[dev-dependencies]`:
`tokio = { workspace = true, features = ["test-util"] }` (the same pattern as
`crates/rpc/Cargo.toml` l.25).

### Step 7 — integration tests: `crates/coordinator_iroh/tests/info_endpoint.rs` (new)

All listeners bind `127.0.0.1:0` (no port literals). Turn the DHT off
(`config.substrate.enable_bep0044_dht = false`). Set
`info_http_bind_address: Some("127.0.0.1:0".into())`. Bound every wait with a
deadline and poll every 50-100 ms. Use the crate-level allow line that the
other test files use (`#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`).

Shared helper: move `spawn_global_registry` from `multi_hop_relay.rs`
(l.274-291) into `tests/common/mod.rs` as
`pub async fn spawn_registry(base_path: &Path) -> Result<(EcosystemRegistry, String)>`,
and call it from both files. `multi_hop_relay.rs` is in
`xtask/oversized-test-files.txt` (cap 1128). Its line count changes with this
move and the import change. Set its entry to the new real count
(`wc -l`), because the ratchet fails in both directions.

A small helper in the new file:

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

**7a. `relay_client_count_follows_connections_and_capacity`** (AC-6 server
side, AC-17 first half):

1. Start a coordinator: `enable_relay = true`, `http_bind_address` and
   `quic_bind_address` `127.0.0.1:0`, `max_relay_clients = Some(2)`, no
   registry. `common::wait_for_relay(&coord, 20s)`.
2. Wait until `relay.clients == Some(1)` and `status == Healthy`. The one
   client is the coordinator's own endpoint (D-A3-8).
3. Bind a client endpoint on the coordinator's relay:
   ```rust
   let relay_url: RelayUrl = info.relay_url.unwrap().parse()?;
   let client = Endpoint::empty_builder()
       .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)))
       .bind().await?;
   time::timeout(Duration::from_secs(20), client.online()).await?;
   ```
4. Wait until `relay.clients == Some(2)` and `status == AtCapacity`, and
   `relay.max_clients == Some(2)`.
5. `client.close().await`. Wait until `relay.clients == Some(1)` and
   `status == Healthy`.
6. `coord.shutdown().await`.

If step 2 never reaches 1, check first whether the coordinator's endpoint
connects to its own relay at all (log `relay.clients`); do not loosen the
test to `>= 1` without finding out why.

**7b. `controller_status_is_verified_after_registration`** (OQ-4 fix):

1. `common::spawn_registry(tmp)`.
2. Start a coordinator with `enable_relay = true`,
   `community_registry_url = Some(r_url)`, `share_in_registry = true`.
3. Wait (20 s deadline) until
   `identity.controller_status == Verified`.
4. Expect `identity.did` to start with `did:key:` and to differ from
   `substrate_id`.

**7c. `probes_do_not_trigger_registry_lookups`** (AC-17 second half):

1. Start a mock registry: an `axum::Router` on `127.0.0.1:0` with
   `POST /register` → `200`, and `GET /lookup/{id}` → add 1 to an
   `Arc<AtomicUsize>` and answer `404`. (Use the same axum path syntax as
   `crates/community_registry/src/registry.rs:135`.)
2. Start a coordinator with `enable_relay = false`,
   `community_registry_url = Some(mock_url)`, `share_in_registry = true`.
3. Wait until the lookup counter is 1 (10 s deadline). This is the first
   check after registration.
4. Send 20 `GET /v1/info` requests. Each must answer within 2 s
   (`time::timeout`). Each answer has `controller_status == Unverified`.
5. Expect the lookup counter to still be 1. (The next check is 60 s away,
   `RECORD_CHECK_UNVERIFIED`, and the test takes a few seconds.)

## 4. Things that are ambiguous or stale in task.md and the change docs

For A3:

1. **The lookup id is wrong today, so `controller_status` is always
   `unverified`.** Not mentioned in task.md or the change doc. See OQ-4.
2. **No config key for the relay client cap.** AC-6 says how to count, not
   what to compare with. See OQ-1.
3. **"at_capacity" cannot be enforced with the `iroh-relay` 0.97 API.** See
   OQ-2.
4. **AC-17 "cached" lookup.** The plan removes the lookup from the handler
   instead (D-A3-4). This meets the criterion and is stronger. Record as a
   deviation.
5. **Where the wire type lives.** Neither task.md nor the change doc says
   that A4 needs `CoordinatorInfo` from a crate the substrate always links.
   See OQ-3.
6. **AC-6 is mostly substrate side.** A3 does only the server half (count and
   `at_capacity`). Leave AC-6's Test column with the server tests and the note
   "substrate skip in A4", so A4's close-out does not skip it.
7. **task.md §3 says A3 runs "Parallel with A2".** A2 is done but not merged.
   See the branch rule in §0.

Stale facts in task.md §1 that later slices rely on (fix them in the same
docs commit):

- "The info server state already holds the host `substrate_id`
  (`crates/coordinator_iroh/src/info_endpoint.rs`), which is the passport
  root." Wrong. `InfoState.substrate_id` is the coordinator's own Iroh
  endpoint id, from a key made at random at every start
  (`coordinator.rs:203`). It is not the host substrate's DID and is not
  stable. The operator-passport change doc already says this ("A coordinator
  role has no stable DID of its own"). B2 must pass the host substrate DID (or
  its controller DID) into the info server itself. Rewrite the bullet to say
  that.
- B2: "*Unverified:* the test harness can run the info server with TLS." It
  can: `crates/coordinator_iroh/tests/tls_rotation.rs` runs the info server
  with `[tls]` and calls `https://{info_addr}/v1/info` (l.109). Rewrite as
  verified, with that file as the evidence.
- The operator-passport change doc cites `coordinator.rs:193-200` for the
  random key. It is `coordinator.rs:203` today, and will move again with this
  slice's refactor (into `build_iroh_endpoint`). Cite the function name, not
  the line.

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
     (`relay_client_count_follows_connections_and_capacity`),
     `crates/coordinator_iroh/src/info_endpoint/tests.rs`; substrate skip in
     A4".
   - AC-17 Test column: `crates/coordinator_iroh/tests/info_endpoint.rs`
     (`relay_client_count_follows_connections_and_capacity`,
     `probes_do_not_trigger_registry_lookups`),
     `crates/coordinator_iroh/src/info_endpoint/tests.rs`
     (`record_check_runs_on_its_own_timer`).
   - Tick the task "Coordinator info endpoint: relay client count for
     `at_capacity`, cache the registry lookup."
   - `## Deviations`: D-A3-1 to D-A3-9, one line each, plus any change you
     made to this plan.
   - Add to `living-docs-touched` every living doc you change (it already
     lists `docs/system-architecture.md`). Keep `status: in-progress`.
3. **Living docs.**
   - `docs/system-architecture.md`:
     - the paragraph that starts "`GET /v1/info` on a coordinator returns"
       (~l.379): say that `status` is `at_capacity` when the relay has at
       least `max_relay_clients` clients and `healthy` otherwise; that
       `relay.clients` is the current relay client count and includes the
       coordinator's own endpoint when it uses its own relay; that the cap is
       reported, not enforced; that `connections` is the Syneroym connection
       count and its cap `max_connections`; that `identity.controller_status`
       comes from a background check of the registry (every 60 s until found,
       then every 10 min), only for a coordinator that registers itself, and
       never from the probe; that `identity.did` is the DID it registers
       under.
     - config table (~l.355-374): add
       `[roles.coordinator.iroh] max_relay_clients` | None (no cap) | "Relay
       clients at which `/v1/info` reports `at_capacity`. Reported only; the
       relay still admits more clients."
   - `docs/system-requirements-spec.md`, `docs/developer-guide.md`,
     `docs/TERMINOLOGY.md`, `docs/planning/traceability-matrix.md`: no change
     expected. Check once (`git grep -n 'v1/info'`).
4. **Status and backlog.**
   - `status.md`: add an A3 row (Done, the tool that implemented it, the
     `mise run verify` result and the test files).
   - `docs/planning/deferred-backlog.md`:
     - new row "The relay client cap is reported, not enforced"
       (D-A3-2: `iroh-relay` 0.97 has no client limit; the access closure
       could refuse, but it runs before a reconnect replaces the old
       connection, and B2 rewrites it). Target `TBD`. Link
       `crates/coordinator_iroh/src/info_endpoint.rs`.
     - the row "A coordinator registers itself once, so its registry record
       expires after 2 hours" (~l.275): add that `controller_status` turns
       `unverified` when this happens, and that the record check task added
       here is the natural place to register again. Update its file links
       (the registration code moves into `start_endpoint` and
       `register_in_global_registry`).
     - a row for any `TODO`/`FIXME` you leave.
   - `task.md` §1: fix the stale bullets listed in §4 of this plan.
5. **Stop.** Do not start A4 or B2. If you are not making progress after a
   reasonable number of attempts, stop and report what blocks you, with the
   failing output.
