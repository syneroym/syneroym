# M06D slice B2 — Relay passport, coordinator side: implementation plan

*Reader: the developer (or AI) who implements slice B2. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "B2", "B3",
  "X7", "P-7", "H-3", "D-B2-4" in code, comments or test names; ADR ids are
  fine), simple English in comments and docs, the 800-line file cap and the
  oversized-test-file ratchet, the 100-line function cap, match arms of at
  most ~10 lines, the module layout rule (`foo.rs` beside `foo/`, never
  `mod.rs` under `src/`), and no hardcoded ports in tests.
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 rows X4 and X7, §2 items 1 to 4 and 6, §3 slices B1, B2 and B3 (B3 is
  the next user of this code — do not build it).
- `docs/planning/changes/operator-passport/change.md`: all of "Core", all of
  "Use: home relay", and all of "Deviations" (`D-B1-1` to `D-B1-28`; several
  are requirements on B2: `D-B1-15`, `D-B1-18`, `D-B1-21`, `D-B1-24`, the
  help-line removal).
- `docs/decisions/0015-ucan-capability-model.md`: §2, A7 and **A10**.
- `crates/passport/src/*` (the core this slice wires in; read
  `registrar.rs`, `lease_store.rs`, `wire.rs`, `limiter.rs`, `error.rs`).
- §2 and §4 of this plan. The owner answered every open question on
  2026-10-15 (`D-B2-3`, `D-B2-6`, `D-B2-15`); `D-B2-1` follows ADR-0015
  A10 as written.

**Branch.** The order is strictly serial (one slice per PR, each merged
before the next branches). B1 is on `feat/m06d-b1-implementation` (PR #306).
Branch `feat/m06d-b2-implementation` from `main` after #306 merges. This
plan is the first commit on that branch.

### Terms used in this plan

- **Passport**, **child token**, **subject**, **lease**, **operator root**,
  **host node DID**: as in the B1 plan and the passport change doc. For the
  relay, the subject is the node DID of the user's substrate, which is also
  its Iroh endpoint key.
- **Info server**: the coordinator's HTTP(S) server that serves `/v1/info`
  (`spawn_http_info_server` in `crates/coordinator_iroh/src/coordinator.rs`).
  It runs HTTPS when the substrate-level `[tls]` is set. The relay's own
  certificate (`[roles.coordinator.tls]`) is a different setting and is not
  used here.
- **Passport access**: `[roles.coordinator] access = "passport"` or
  `access = { list = [...], passport = true }`.
- **Passport root**: the one DID a passport may be issued by (ADR-0015 A10
  point 4): the controller of a verified `ControllerAgreement` of the host
  substrate, or the host node DID when the host has no verified controller.
  Never `[iam].admin_ucan_root`.
- **Admin root**: the DID that holds `substrate/admin` on a substrate. It is
  the controller of a verified `ControllerAgreement`, else
  `[iam].admin_ucan_root`, else none (the substrate is unowned). This is the
  existing rule in `setup_connection_router`
  (`crates/substrate/src/runtime/router.rs:64-77`). It decides who may run
  `deny` and `leases` (`D-B2-6`). It is not a passport root.
- **Always-admitted endpoints**: the coordinator's own Iroh endpoint and the
  host substrate's node endpoint (§2, `D-B2-3`).
- **Visitor**: a peer that connects to a relay only to reach a node whose
  home relay it is. Iroh opens such "non-home relay" connections itself
  (`iroh-0.97.0/src/socket/transports/relay/actor.rs:64`), and the relay's
  access check applies to them (`D-B2-15`).

## 1. What this slice delivers

The coordinator accepts passports. After B2, an operator can run a relay with
`access = "passport"`, a substrate (by hand, or by B3 later) can register
over HTTPS and is admitted, and the operator can deny a passport and list the
leases on the running relay.

1. `crates/passport`: two more refusal codes (`rate_limited`,
   `not_accepted`), the two endpoint paths as constants, and a `LeaseInfo`
   wire type for the operator's lease list.
2. `crates/core`: `CoordinatorAccess::accepts_passports`, `list` and
   `check_tls`, and a `HostIdentity` type in a neutral module
   (`host_identity`): the host node DID, its controller status, and its
   verified controller. It gives the passport root of A10. The relay uses it
   now; the dedicated mailbox (C8) reuses it (`D-B2-16`).
3. `crates/router`: `ucan_chain_not_revoked` made public, so the coordinator
   can run the DHT revocation check (`D-B1-15`).
4. `crates/coordinator_iroh`:
   - The strict access check at startup now accepts the passport forms and
     requires `[tls]` with them (`P-10`).
   - A relay with passport access opens a lease store
     (`<storage.db_dir>/relay_passports.db`) and a `Registrar` for
     `RELAY_HOME`, rooted at the passport root of A10.
   - `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register` on
     the info server, with the request limiter, the 16 KiB body limit, the
     revocation check between `check` and `grant`, and JSON error bodies.
   - The relay access function reads the lease table at each connect.
   - The coordinator's own endpoint and the host node are always admitted
     under every restricted form.
   - `/v1/info` reports `identity` (host node DID and controller status).
5. `crates/coordinator`, `crates/substrate`: the host identity is loaded
   before the coordinator starts and passed in; the coordinator's lease store
   is handed to the control plane.
6. `crates/control_plane`: two node-owner methods on the `security`
   interface, `relay-passport-deny` and `relay-passport-leases`.
7. `crates/sdk`, `apps/roymctl`: `roymctl coordinator passport deny <id>` and
   `roymctl coordinator passport leases`, through the running substrate
   (`D-B1-18`). The "A relay cannot use passports yet" help line is removed.
8. Tests, living docs, the change doc, `status.md`, `task.md` and the
   deferred backlog.

Not in this slice (B3): the substrate's child-token store, `roymctl substrate
home-relay set` and `status`, registering before the first relay connect,
renewal, the 7-day warning and metric, and the full restricted-relay
integration test with a real substrate registering itself.

## 2. Decisions this plan takes

Write each one into the passport change doc's `## Deviations` as `D-B2-n`
(see Step 13). The owner confirmed `D-B2-3`, `D-B2-6` and `D-B2-15` on
2026-10-15. No owner check is open.

- `D-B2-1`: **the passport root follows ADR-0015 A10 point 4.** The relay
  accepts a passport rooted at the verified controller of the host
  substrate, or at the host node DID when the host has no verified
  controller. Exactly one root, never both, and never
  `[iam].admin_ucan_root` (A10: "It is not the node-wide
  `admin_ucan_root`"). An unverified controller is not a controller (the
  same rule as `substrate/admin`). So on a host with `admin_ucan_root` and no
  verified controller, the node key issues passports and the admin root
  denies them (`D-B2-6`); on a host with a verified controller, both are the
  controller. The root is read once at startup; a controller agreement that
  ends or changes while the substrate runs keeps the old root until restart
  (the same as `substrate/admin`, see the `roymctl substrate claim
  --expires-days` help). Backlog row. `roymctl coordinator passport issue
  --node-key` keeps its help text ("only for a host substrate that has no
  controller"), which is right under this rule.
- `D-B2-2`: **`[tls]` is required whenever `access` accepts passports**, for
  every coordinator role: also when `enable_relay = false`, and also when
  the role has no `[roles.coordinator.iroh]` section (only WebRTC, or
  nothing). The check runs in `EcosystemCoordinator::init` before any
  coordinator is built, and again in `CoordinatorIroh::init` (which tests
  and `smoke-tests` call directly). Both call one function,
  `CoordinatorAccess::check_tls` in `crates/core`. The error names `[tls]`.
  When the role runs no Iroh relay, passport access only logs a warning
  ("has no effect"), opens no lease store, and the endpoints (if the info
  server runs) answer `not_accepted`.
- `D-B2-3` (owner confirmed): **the coordinator's own endpoint and the host
  node are always admitted, under every restricted form** (list, passport,
  both). task.md asks "whether the coordinator's own endpoint is admitted on
  its own relay with passport access". Without this, the coordinator's
  random endpoint has no lease and no home relay. The host node has the same
  problem: a substrate that runs a passport relay and uses it as its own home
  relay (the test harness does this) would be refused by its own relay. One
  rule for all restricted forms is simpler, and it also fixes the same gap
  for the list form. It **replaces `D-A3-10`** of the coordinators change
  doc ("The coordinator's own endpoint is counted toward relay clients only
  when admitted by relay access (e.g. `access = "everyone"`)"): now it is
  counted whenever it uses its own relay (no parent relay). It changes the
  `relay.clients` count of a list-access relay by one; one existing test
  changes (Step 12.4), and the `max_relay_clients = 1` warning of
  `check_relay_cap` now fires for every access form (Step 6.5). Consequence
  for B3: a test whose node uses its own relay is always admitted, so it
  would pass even if registration did nothing. B3's tests must register a
  node with a *separate* passport relay node (Step 13.1, requirements on
  B3).
- `D-B2-4`: **two more refusal codes in the core**, `RegisterError::RateLimited`
  (`rate_limited`) and `RegisterError::NotAccepted` (`not_accepted`).
  `D-B1-15` says the error set is final; it is not, because `H-3` ("refuses
  registration with a clear error") and the request limit (`P-7`) need
  codes. Keeping every wire code in one enum lets B3 match on one list.
- `D-B2-5`: **HTTP status of each refusal.** `request_too_large` 413;
  `malformed`, `unknown_challenge`, `challenge_expired` 400; `rate_limited`
  429; `internal` 500; every other code 403 (`bad_signature`, `not_bound`,
  `untrusted_root`, `broken_chain`, `wrong_capability`, `expired`,
  `invalid_chain`, `denied`, `revoked`, `limit_reached`, `not_accepted`). A
  client decides by `code`, not by status.
- `D-B2-6` (owner confirmed): **`deny` and `leases` go through the control
  plane**, as two methods of the existing `security` interface, gated by
  `substrate/admin` like the other methods there. They are declared in the
  WIT `security` interface, like every other control-plane method (Step
  10.4). Rejected: signed admin requests on the info server. That needs a
  new signed-request format and replay rule, and it would be a second admin
  path beside the control plane. The control plane is already how `roymctl`
  runs every node-owner verb, and B3 uses it too. Cost: an unowned host (no
  controller and no `[iam].admin_ucan_root`) cannot run `deny` or `leases`,
  although its node key can issue passports. This matches the rest of the
  system (an unowned substrate refuses every admin verb). Backlog row.
- `D-B2-7`: **the lease file** is `<storage.db_dir>/relay_passports.db`. The
  directory is created if missing.
- `D-B2-8`: **the revocation lookup uses the host's registry**:
  `RegistryClient::new(config.substrate.enable_bep0044_dht,
  config.substrate.registry_url.clone())`, the same source the node's own
  router uses for every other UCAN chain
  (`crates/router/src/route_handler.rs:372`). Not the coordinator's
  `[roles.coordinator.iroh] community_registry_url`, which is where the
  coordinator registers itself. A failed lookup counts as "not revoked"; this
  is the existing behaviour of `ucan_chain_not_revoked`. Backlog row.
- `D-B2-9`: **the request limiter counts the TCP peer address.**
  `X-Forwarded-For` is not read: anyone could set it. Behind a reverse proxy,
  all users share the proxy's address and one limit. Backlog row.
- `D-B2-10`: **one request limiter for both endpoints.** A registration costs
  two requests (challenge, register), so one address can register five times
  a minute. A `not_accepted` answer does not count (no limiter exists then).
- `D-B2-11`: **the challenge request body is ignored.** `POST
  /v1/home-relay/challenge` takes no body. Its answer is `ChallengeResponse`
  unchanged.
- `D-B2-12`: **body reading.** The register route has
  `DefaultBodyLimit::max(MAX_REQUEST_BYTES)`, and the handler takes
  `Result<Bytes, BytesRejection>`, so an oversize body gets the JSON
  `request_too_large` body (status 413) and not axum's plain-text one. The
  body is read (at most 16 KiB) before the limiter runs; the size and shape
  checks still run before any signature check.
- `D-B2-13`: **`/v1/info` `identity` is always reported now**:
  `{ "did": <host node DID>, "controller_status": "verified" |
  "unverified" | "none" }` (the M02 meaning; the values are
  `SubstrateIdentityStatus` in lower case).
- `D-B2-14`: **`max_lease_secs` is not configurable.** The relay uses the 24
  hours of `RELAY_HOME`. No config key (no user asked for one).
- `D-B2-15` (owner confirmed): **a passport relay is a closed-group relay.**
  The Iroh relay checks access for every client that connects, and a peer
  that sends to a node through that node's home relay connects to it as a
  client (a visitor). The access function sees only the endpoint id, so it
  cannot tell a home client from a visitor. So on a relay with passport (or
  list) access, only leased, listed or always-admitted endpoints can reach
  each other through it. Other substrates, `roymctl` and `SyneroymClient`
  reach a leased node only through a direct path, or if they hold a lease
  too. This goes beyond `H-1` (which only says the leased node may
  connect). It is written in the requirements and architecture docs and
  the operator note (Step 13.2), and as a backlog row: "Visitors need a lease
  on a passport relay; needs a relay access hook that tells home clients
  from visitors (iroh-relay 0.97 has none)".
- `D-B2-16`: **the root rule lives in a neutral module.** `HostIdentity` and
  its `passport_root()` go in `crates/core/src/host_identity.rs`, not in
  `coordinator_info`, because A10's per-service root rule is shared with the
  dedicated mailbox. C8 reuses `HostIdentity::passport_root` for
  `MAILBOX_USE` and must not write its own. `/v1/info` gets its
  `IdentityInfo` from `HostIdentity::identity_info`.
- `D-B2-17`: **`LeaseInfo` has no `limits`.** The WIT record of the lease
  list (Step 10.4) has no JSON map type, and the operator issued the limits
  and knows them. The lease row still keeps them (`D-B1-16`) for the
  mailbox quota lookup.

## 3. Step-by-step changes

Steps 1 to 4 are small library changes. Steps 5 to 7 are the coordinator.
Steps 8 to 11 wire it through the substrate and `roymctl`. Step 12 is the
tests, Step 13 the docs.

### Step 1 — `crates/passport`

**1.1 `src/error.rs`.** Add two variants and their codes:

```rust
    #[error("too many requests from this address; try again in a minute")]
    RateLimited,
    #[error("this service does not accept passports")]
    NotAccepted,
```

In `code()`: `Self::RateLimited => "rate_limited"`, `Self::NotAccepted =>
"not_accepted"`. Put `NotAccepted` first in the enum (it is checked first)
and `RateLimited` second. Update the doc comment of `Revoked` only if it says
the set is final (it does not today). Extend `src/error/tests.rs` so the list
of every code includes the two new ones and stays unique.

**1.2 `src/wire.rs`.** Add, after `MAX_REQUEST_BYTES`:

```rust
/// The path of the challenge endpoint on the coordinator's info server.
pub const CHALLENGE_PATH: &str = "/v1/home-relay/challenge";
/// The path of the register endpoint on the coordinator's info server.
pub const REGISTER_PATH: &str = "/v1/home-relay/register";
```

Add the operator's lease list type (the module doc gains one sentence: "It
also holds `LeaseInfo`, the lease list the operator reads through the running
service."):

```rust
/// One running lease, as the operator lists it. Mirrors the WIT record
/// `relay-lease` of the control-plane `security` interface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseInfo {
    pub passport_id: PassportId,
    pub owner_did: String,
    pub subject: String,
    pub lease_ends_at: u64,
    pub registered_at: u64,
}

impl From<&Lease> for LeaseInfo { /* copy the five fields */ }
```

`use crate::lease_store::Lease;`. No `limits` (`D-B2-17`). `LeaseInfo`
accepts unknown fields (no `deny_unknown_fields`), like the other response
types (`D-B1-24`).

**1.3 `src/lib.rs`.** `wire` is already `pub`. No new re-export is needed;
callers use `syneroym_passport::wire::{...}`.

**1.4 tests.** `src/wire/tests.rs`: one test that a `LeaseInfo` made from a
`Lease` round-trips through JSON, and that an extra field in the JSON is
accepted.

### Step 2 — `crates/core`

**2.1 `src/config/access.rs`.** Add to `impl CoordinatorAccess`:

```rust
impl CoordinatorAccess {
    /// Whether a node with a running passport lease is admitted.
    #[must_use]
    pub const fn accepts_passports(&self) -> bool {
        matches!(self, Self::Passport | Self::ListOrPassport(_))
    }

    /// The listed endpoint ids. Empty for `Everyone` and `Passport`.
    #[must_use]
    pub fn list(&self) -> &[String] {
        match self {
            Self::List(ids) | Self::ListOrPassport(ids) => ids,
            Self::Everyone | Self::Passport => &[],
        }
    }

    /// The passport forms need `[tls]`: the passport endpoints carry the
    /// passport and are served only over HTTPS.
    ///
    /// # Errors
    /// For a passport form when `has_tls` is false.
    pub fn check_tls(&self, has_tls: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            has_tls || !self.accepts_passports(),
            "[roles.coordinator] access accepts passports, so [tls] must be set: the passport \
             endpoints are served only over HTTPS"
        );
        Ok(())
    }
}
```

Check that `crates/core` already depends on `anyhow` (it does for other
config checks; if not, return a `String` error instead). Change the enum
doc: replace "A coordinator may refuse to start with a form it cannot
enforce yet; parsing never widens a value to "everyone"." with "Parsing
never widens a value to "everyone". The passport forms need `[tls]`
(`check_tls`)." Add short tests to `src/config/access/tests.rs` for the
three methods (`check_tls`: both passport forms with and without TLS, and
`Everyone` and a list without TLS pass).

Leave the comment on `AccessControl` in `src/config/roles.rs:62-63` alone:
it is the community registry's setting, not the coordinator's.

**2.2 `src/host_identity.rs` (new; `pub mod host_identity;` in
`src/lib.rs`).** The host identity and the passport root rule of ADR-0015
A10 point 4, shared by the relay now and the dedicated mailbox (C8) later
(`D-B2-16`). Not a wire type.

```rust
//! The identity of the substrate that runs an infrastructure service (a
//! relay, a mailbox), and the passport root it accepts (ADR-0015 A10).

use syneroym_identity::substrate::{SubstrateIdentityState, SubstrateIdentityStatus};

use crate::coordinator_info::IdentityInfo;

/// The host substrate of a service, as read at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentity {
    /// The node DID of the host substrate.
    pub node_did: String,
    /// The state of its controller agreement.
    pub controller_status: SubstrateIdentityStatus,
    /// The controller of a verified agreement. `None` when there is no
    /// agreement or it is not verified.
    pub verified_controller: Option<String>,
}

impl HostIdentity {
    /// From the boot identity state. A controller counts only when its
    /// agreement is verified.
    #[must_use]
    pub fn new(state: &SubstrateIdentityState) -> Self {
        let verified_controller = (state.status == SubstrateIdentityStatus::Verified)
            .then(|| state.controller.clone())
            .flatten();
        Self {
            node_did: state.did.clone(),
            controller_status: state.status.clone(),
            verified_controller,
        }
    }

    /// A host with no controller.
    #[must_use]
    pub fn unowned(node_did: String) -> Self {
        Self {
            node_did,
            controller_status: SubstrateIdentityStatus::None,
            verified_controller: None,
        }
    }

    /// The one DID a passport for a service of this host may be issued by
    /// (ADR-0015 A10 point 4): the verified controller, or the node DID
    /// when there is none. Never the node-wide `[iam].admin_ucan_root`.
    #[must_use]
    pub fn passport_root(&self) -> &str {
        self.verified_controller.as_deref().unwrap_or(&self.node_did)
    }

    /// What `/v1/info` reports as `identity`.
    #[must_use]
    pub fn identity_info(&self) -> IdentityInfo {
        let status = match self.controller_status {
            SubstrateIdentityStatus::Verified => "verified",
            SubstrateIdentityStatus::Unverified => "unverified",
            SubstrateIdentityStatus::None => "none",
        };
        IdentityInfo { did: self.node_did.clone(), controller_status: status.to_string() }
    }
}

/// A host with a fresh random node key and no controller, for tests.
#[cfg(any(test, feature = "test-utils"))]
impl HostIdentity {
    #[must_use]
    pub fn random_unowned() -> Self {
        let id = Identity::generate().expect("random identity");
        Self::unowned(substrate::derive_did_key(&id.public_key()))
    }
}

#[cfg(test)]
mod tests;
```

Imports per the import rule (`use syneroym_identity::{Identity, substrate};`
under the same `cfg`, merged with the line above if rustfmt prefers).
`expect` in a `test-utils` function: check how other `test-utils` items in
`crates/core` handle the `expect_used` lint, and do the same (an
`#[expect(..., reason)]` on the function, or return `anyhow::Result`).

In `src/coordinator_info.rs`, change the `IdentityInfo` doc from "Not
reported yet: the field is `null`." to "The identity of the host substrate:
its node DID and the state of its controller agreement (`verified`,
`unverified` or `none`)."

Tests in `src/host_identity/tests.rs`: `new` with a verified controller
(root is the controller, not the node DID), with an unverified controller
(root is the node DID, `verified_controller` is `None`), and with none;
`identity_info` gives the three lower-case strings.

The substrate keeps its own admin-root rule (verified controller, else
`[iam].admin_ucan_root`) in `setup_connection_router`; Step 9.1 reads the
verified controller from `HostIdentity` for it, so the identity is read once.

### Step 3 — `crates/router`: make the revocation check reachable

`crates/router/src/route_handler/io.rs:69`: `async fn ucan_chain_not_revoked`
becomes `pub async fn ucan_chain_not_revoked`. Its doc comment stays. In
`crates/router/src/lib.rs`, add
`pub use route_handler::io::ucan_chain_not_revoked;` next to the other
`pub use route_handler::...` line. `route_handler::io` is already `pub mod`
inside a private `mod route_handler`, so this re-export is the only public
path. No behaviour change.

### Step 4 — `crates/coordinator_iroh/Cargo.toml`

Add to `[dependencies]`: `syneroym-passport.workspace = true`,
`ed25519-dalek.workspace = true` (to turn an `EndpointId` into a DID; iroh's
own `as_verifying_key` returns the pre-release `ed25519-dalek` 3 type, which
`syneroym_identity` does not accept). Add to `[dev-dependencies]`:
`syneroym-core = { workspace = true, features = ["test-utils"] }` (for
`HostIdentity::random_unowned`). `async-trait` is already a dev-dependency
(the unit test's mock resolver needs it). Check `cargo shear` afterwards.

### Step 5 — `crates/coordinator_iroh/src/passport.rs` (new) and `src/passport/tests.rs`

Add `mod passport;` in `src/lib.rs`. The module:

```rust
//! The passport side of a relay with passport access: the lease store, the
//! two registration endpoints on the info server, and the DID of a relay
//! client.

use std::{fs, net::{IpAddr, SocketAddr}, path::Path, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, State,
              rejection::{BytesRejection, FailedToBufferBody}},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing,
};
use ed25519_dalek::VerifyingKey;
use iroh::EndpointId;
use syneroym_core::{host_identity::HostIdentity, dht_registry::MasterAnchorResolver};
use syneroym_identity::substrate;
use syneroym_passport::{
    CHALLENGE_LIFE_SECS, Lease, LeaseStore, RELAY_HOME, RegisterError, Registrar,
    RequestLimiter, SignerBinding,
    wire::{self, ChallengeResponse, ErrorBody, RegisterRequest, RegisterResponse},
};
use tokio::task;
use tracing::{debug, error, info};

/// The lease file of the relay, in `storage.db_dir`.
pub(crate) const RELAY_LEASES_DB: &str = "relay_passports.db";

/// What the registration endpoints of one relay need.
pub(crate) struct RelayPassports {
    registrar: Arc<Registrar>,
    limiter: RequestLimiter,
    resolver: Arc<dyn MasterAnchorResolver>,
}
```

Write a manual `Debug` for `RelayPassports` (the resolver is `Debug` by its
trait bound, so `#[derive(Debug)]` may work; use it if it compiles).

```rust
impl RelayPassports {
    /// Open the lease store in `db_dir` and the registrar for the home-relay
    /// use, rooted at the host's passport root (ADR-0015 A10 point 4).
    pub(crate) async fn open(
        db_dir: &Path,
        host: &HostIdentity,
        resolver: Arc<dyn MasterAnchorResolver>,
    ) -> Result<Self> {
        let dir = db_dir.to_path_buf();
        let leases = task::spawn_blocking(move || {
            fs::create_dir_all(&dir)?;
            LeaseStore::open(&dir, RELAY_LEASES_DB, &RELAY_HOME, syneroym_passport::now_secs())
        })
        .await
        .context("the relay lease store task failed")?
        .context("could not open the relay lease store")?;
        let root = host.passport_root().to_string();
        let registrar =
            Registrar::new(RELAY_HOME, host.node_did.clone(), vec![root.clone()], leases)
                .context("[roles.coordinator] access accepts passports")?;
        info!(%root, "the relay accepts passports issued by this DID");
        Ok(Self { registrar: Arc::new(registrar), limiter: RequestLimiter::default(), resolver })
    }

    pub(crate) fn leases(&self) -> &LeaseStore {
        self.registrar.leases()
    }

    fn challenge(&self, peer: IpAddr, now: u64) -> Result<ChallengeResponse, RegisterError> {
        if !self.limiter.allow(peer, now) {
            return Err(RegisterError::RateLimited);
        }
        let challenge = self
            .registrar
            .issue_challenge(now)
            .map_err(|e| RegisterError::Store(format!("{e:#}")))?;
        Ok(ChallengeResponse { challenge: challenge.to_hex(), expires_in_secs: CHALLENGE_LIFE_SECS })
    }

    /// Limit, size and shape, the offline checks, the DHT revocation check
    /// on every chain edge, then the lease write.
    async fn register(
        &self,
        peer: IpAddr,
        body: Result<Bytes, BytesRejection>,
        now: u64,
    ) -> Result<Lease, RegisterError> {
        if !self.limiter.allow(peer, now) {
            return Err(RegisterError::RateLimited);
        }
        let body = body.map_err(body_error)?;
        let (token, proof) = RegisterRequest::from_slice(&body)?.into_parts()?;
        let checked = self.registrar.check(&token, &proof, &SignerBinding::SameKey, now)?;
        // ADR-0015 §2 and A7: a revoked edge stops the registration. The
        // lookup needs the network, so it runs only after every offline
        // check passed.
        if !syneroym_router::ucan_chain_not_revoked(checked.token(), self.resolver.as_ref()).await {
            return Err(RegisterError::Revoked);
        }
        let registrar = Arc::clone(&self.registrar);
        // A fresh clock: the lookup above may take seconds.
        let lease = task::spawn_blocking(move || registrar.grant(checked, syneroym_passport::now_secs()))
            .await
            .map_err(|e| RegisterError::Store(format!("the lease write task failed: {e}")))??;
        info!(passport_id = %lease.passport_id, subject = %lease.subject,
              lease_ends_at = lease.lease_ends_at, "relay passport lease granted");
        Ok(lease)
    }
}

fn body_error(e: BytesRejection) -> RegisterError {
    match e {
        BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
            RegisterError::RequestTooLarge { max: wire::MAX_REQUEST_BYTES }
        }
        other => RegisterError::Malformed(format!("could not read the request body: {other}")),
    }
}
```

The routes. The state is `None` when the relay does not accept passports
(the access form, or `enable_relay = false`); both endpoints then answer
`not_accepted` (`H-3`).

```rust
type PassportState = Option<Arc<RelayPassports>>;

/// `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.
pub(crate) fn routes(passports: PassportState) -> Router {
    Router::new()
        .route(wire::CHALLENGE_PATH, routing::post(post_challenge))
        .route(wire::REGISTER_PATH, routing::post(post_register))
        .layer(DefaultBodyLimit::max(wire::MAX_REQUEST_BYTES))
        .with_state(passports)
}

async fn post_challenge(
    State(passports): State<PassportState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    let Some(passports) = passports else { return refusal(&RegisterError::NotAccepted) };
    respond(passports.challenge(peer.ip(), syneroym_passport::now_secs()))
}

async fn post_register(
    State(passports): State<PassportState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(passports) = passports else { return refusal(&RegisterError::NotAccepted) };
    let result = passports.register(peer.ip(), body, syneroym_passport::now_secs()).await;
    respond(result.map(|lease| RegisterResponse {
        passport_id: lease.passport_id,
        lease_ends_at: lease.lease_ends_at,
    }))
}

fn respond<T: serde::Serialize>(result: Result<T, RegisterError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(e) => refusal(&e),
    }
}

/// The error body of `e`. A store error is logged here with its full text,
/// because the body carries only "internal error".
fn refusal(e: &RegisterError) -> Response {
    if let RegisterError::Store(_) = e {
        error!(error = %e, "relay passport store error");
    } else {
        debug!(code = e.code(), "relay passport registration refused: {e}");
    }
    (status_of(e), Json(ErrorBody::from(e))).into_response()
}

fn status_of(e: &RegisterError) -> StatusCode {
    match e {
        RegisterError::RequestTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        RegisterError::Malformed(_)
        | RegisterError::UnknownChallenge
        | RegisterError::ChallengeExpired => StatusCode::BAD_REQUEST,
        RegisterError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        RegisterError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::FORBIDDEN,
    }
}
```

If clippy refuses the `_` arm (`wildcard_enum_match_arm` is not in the
workspace lints today; check `Cargo.toml` `[workspace.lints]`), list the
remaining variants explicitly.

The DID helpers:

```rust
/// The DID of a relay client: its endpoint id is its ed25519 key. `None` for
/// bytes that are not a valid key (the relay then refuses it).
pub(crate) fn endpoint_did(id: &EndpointId) -> Option<String> {
    VerifyingKey::from_bytes(id.as_bytes()).ok().map(|key| substrate::derive_did_key(&key))
}

/// The endpoint id of a `did:key` (the host node's Iroh endpoint).
pub(crate) fn endpoint_of_did(did: &str) -> Result<EndpointId> {
    let key = substrate::resolve_did_key(did)?;
    EndpointId::from_bytes(key.as_bytes()).with_context(|| format!("{did} is not an Iroh key"))
}

/// Whether the client `id` has a running lease.
pub(crate) fn is_leased(leases: &LeaseStore, id: &EndpointId) -> bool {
    endpoint_did(id).is_some_and(|did| leases.is_admitted(&did, syneroym_passport::now_secs()))
}
```

Check the exact name and error type of `EndpointId::from_bytes` in the cached
`iroh-base-0.97` source (`src/key.rs:116`, returns `Result<Self,
KeyParsingError>`); adjust `with_context` if the error type does not implement
`std::error::Error + Send + Sync`.

**Unit tests** (`src/passport/tests.rs`, `#[cfg(test)] mod tests;` at the end
of `passport.rs`). Use `tempfile` for `db_dir` and a small mock resolver:

```rust
#[derive(Debug)]
struct Revokes(Option<(String, String)>); // (issuer, revoked audience)

#[async_trait::async_trait]
impl MasterAnchorResolver for Revokes {
    async fn resolve_master_anchor(&self, master_id: &str) -> anyhow::Result<MasterAnchorPayload> {
        // Return a payload whose `revoked_keys` holds the audience when
        // `master_id` is the issuer, else an error (= not revoked).
    }
}
```

Check how `crates/router/src/route_handler/io/tests.rs:31` builds a
`MasterAnchorPayload` and copy the minimal form. Build the request body with
`syneroym_passport::{issue_passport, child_token, sign_challenge}` (the
operator is the host node key, `HostIdentity::unowned(node_did)`). Tests:

1. `register` with a valid body grants a lease; `leases().is_admitted` is
   true for the subject.
2. With the resolver revoking the `(owner, subject)` edge: `Revoked`, and no
   lease is written. Same for the `(operator, owner)` edge.
3. An 11th call from one address in the same second is `RateLimited`
   (call `challenge` 10 times, then the 11th).
4. `body_error` maps a length-limit rejection to `RequestTooLarge` (build
   the rejection by running the route through `tower::ServiceExt::oneshot`
   with a 17 KiB body if a direct value cannot be built; `tower` is a
   workspace dependency — add it as a dev-dependency only if needed).
5. `status_of` gives the statuses of `D-B2-5` for every variant (one table
   test).
6. `endpoint_did(endpoint_of_did(did)) == did` for a generated DID.
7. `open` with a `verified_controller` that is not a canonical DID fails,
   naming `[roles.coordinator] access`.
8. Roots (A10 point 4): with a verified controller, a passport issued by
   the controller is granted and one issued by the node key is
   `UntrustedRoot`; with no controller, the node key's passport is granted.

### Step 6 — `crates/coordinator_iroh/src/config.rs`

**6.1 Startup check.** Replace the early use of `relay_access` with a check
that builds nothing:

```rust
/// Checks `[roles.coordinator] access` at startup.
///
/// # Errors
/// For a list entry that is not an Iroh endpoint id, and for a passport
/// form without `[tls]` (`CoordinatorAccess::check_tls`).
pub fn check_access(access: &CoordinatorAccess, has_tls: bool, relay_on: bool) -> Result<()> {
    let ids = parse_endpoint_ids(access.list())?;
    if ids.is_empty() && matches!(access, CoordinatorAccess::List(_)) {
        warn!("[roles.coordinator] access is an empty list; only the coordinator's own endpoint \
               and the host node may use this relay");
    }
    access.check_tls(has_tls)?;
    if access.accepts_passports() && !relay_on {
        warn!("[roles.coordinator] access accepts passports, but enable_relay is false; it has \
               no effect");
    }
    Ok(())
}

fn parse_endpoint_ids(ids: &[String]) -> Result<Vec<EndpointId>> {
    // the body of today's `restricted_to` map/collect, unchanged
}
```

**6.2 The access function.** New signature:

```rust
/// Builds the relay access check from `[roles.coordinator] access`.
/// `always`: endpoints every restricted form admits (the coordinator's own
/// endpoint and the host node). `leases`: the lease store; it must be
/// `Some` exactly when `access` accepts passports.
///
/// # Errors
/// For a list entry that is not an Iroh endpoint id, or a lease store that
/// does not match `access`.
pub fn relay_access(
    access: &CoordinatorAccess,
    always: &[EndpointId],
    leases: Option<LeaseStore>,
) -> Result<AccessConfig> {
    if *access == CoordinatorAccess::Everyone {
        return Ok(AccessConfig::Everyone);
    }
    ensure!(access.accepts_passports() == leases.is_some(),
            "the relay lease store does not match [roles.coordinator] access");
    let mut admitted = parse_endpoint_ids(access.list())?;
    admitted.extend_from_slice(always);
    let admitted = Arc::new(admitted);
    Ok(AccessConfig::Restricted(Box::new(move |id| {
        let allow = admitted.contains(&id)
            || leases.as_ref().is_some_and(|l| passport::is_leased(l, &id));
        Box::pin(async move { if allow { Access::Allow } else { Access::Deny } })
    })))
}
```

The lease read happens in the closure body, before the future; it reads
memory only (`D-B1-13`). Delete `restricted_to`.

**6.3 `build_relay_config`.** New signature
`build_relay_config(role: &CoordinatorRole, always: &[EndpointId], leases:
Option<LeaseStore>) -> Result<ServerConfig<io::Error>>`. Move the line
`let access_config = relay_access(&role.access)?;` inside the
`if iroh_cfg.enable_relay` branch, as `relay_access(&role.access, always,
leases)?`, so a relay that does not run builds no access function.

**6.4 Unit tests in `config.rs`.** Every existing test that calls
`relay_access` or `build_relay_config` changes, because both signatures
change:

- `test_build_relay_config_enable_relay` and
  `test_build_relay_config_disable_relay`: pass `&[]`, `None`.
- `everyone_admits_any_endpoint`: call
  `relay_access(&CoordinatorAccess::Everyone, &[], None)`.
- `list_admits_only_listed_endpoints`: call with `&[]`, `None`; it keeps its
  assertions (no always-admitted ids, so only the listed one passes). Rename
  it to `list_admits_listed_and_always_admitted_endpoints` and add an
  always-admitted id that is `Allow` too.
- `list_with_bad_id_is_an_error`: becomes `check_access_rejects_bad_ids`
  (the parse error now comes from `check_access`; `relay_access` still
  returns it too).
- `passport_access_is_an_error`, `table_access_is_an_error` and
  `build_relay_config_rejects_passport_access`: delete; they test the old
  refusal.

New tests:

- `passport_access_admits_leased_and_always_admitted_endpoints`: an
  in-memory `LeaseStore::open_in_memory(&RELAY_HOME)`; grant a lease by
  running a real `Registrar` check and grant (same fixture as the roymctl
  tests, `apps/roymctl/src/commands/coordinator/tests.rs:36`), or, simpler,
  test `is_leased` in Step 5 and here only check that an always-admitted id
  is `Allow` and a random id is `Deny`.
- `table_form_admits_listed_or_leased`.
- `everyone_ignores_always_list` (still `AccessConfig::Everyone`).
- `lease_store_must_match_access`: `Passport` with `None` and `List` with
  `Some` are errors.
- `check_access_requires_tls_for_passports` (both passport forms, with and
  without TLS).
- `cap_of_one_warns_for_every_access_form` (Step 6.5; assert on the
  returned value if you make the check return its warning, or leave it to
  review if it only logs).

`config.rs` is 284 lines; with the new tests it stays below 800. If it gets
near 500, move the test module to `src/config/tests.rs`.

**6.5 `check_relay_cap`.** Today the `max_relay_clients = 1` warning
(`config.rs:67-77`) fires only for `access == Everyone`, because only then
did the coordinator's own endpoint take a relay slot (`D-A3-10`). With
`D-B2-3` it takes one under every access form when the coordinator has no
parent relay. New condition: `Some(1) if iroh_cfg.enable_relay &&
!has_parent_relay`. The `access` parameter is then unused: remove it, and
update the one call site in `coordinator.rs` (Step 7.3). Change the warning
text: "max_relay_clients is 1 and there is no parent relay; the
coordinator's own endpoint occupies this slot, so the relay will always
report at_capacity".

### Step 7 — `crates/coordinator_iroh/src/coordinator.rs` and `src/info_endpoint.rs`

**7.1 Signature.** `pub async fn init(config: &SubstrateConfig, host:
&HostIdentity) -> Result<Self>`.

**7.2 New field and getter.**

```rust
pub struct CoordinatorIroh {
    // ... existing fields ...
    /// The lease store and registrar of a relay with passport access.
    relay_passports: Option<Arc<RelayPassports>>,
}

/// The lease store of a relay with passport access, for the operator's
/// `deny` and lease list. `None` when the relay does not accept passports.
#[must_use]
pub fn relay_leases(&self) -> Option<LeaseStore> {
    self.relay_passports.as_ref().map(|p| p.leases().clone())
}
```

Add the field to the manual `Debug` impl (`.field("relay_passports",
&self.relay_passports.is_some())`).

**7.3 The order inside `init`.** `init` is close to the 100-line limit;
extract the body under `if let Some(role) = &config_roles` into
`async fn start_role(config, role, host, parent_relay_url: Option<&RelayUrl>)
-> Result<Self>` and keep `init` short. Pseudo-code of `start_role`:

```rust
let relay_on = role.iroh.as_ref().is_some_and(|i| i.enable_relay);
// Checked even when the relay is off, so a value that cannot work stops
// the coordinator at startup instead of waiting unused.
config::check_access(&role.access, config.tls.is_some(), relay_on)?;
if let Some(iroh_cfg) = &role.iroh {
    config::check_relay_cap(iroh_cfg, parent_relay_url.is_some())?; // Step 6.5
}
// Made before the relay server, so the relay can always admit the
// coordinator's own endpoint.
let secret_key = SecretKey::generate(&mut rand::rng());
let relay_passports = if relay_on && role.access.accepts_passports() {
    let resolver: Arc<dyn MasterAnchorResolver> = Arc::new(RegistryClient::new(
        config.substrate.enable_bep0044_dht,
        config.substrate.registry_url.clone(),
    ));
    Some(Arc::new(RelayPassports::open(&config.storage.db_dir, host, resolver).await?))
} else {
    None
};
let relay_server = if relay_on {
    let always = [secret_key.public(), passport::endpoint_of_did(&host.node_did)?];
    let leases = relay_passports.as_ref().map(|p| p.leases().clone());
    Some(Self::spawn_relay_server(role, &always, leases).await?)
} else {
    None
};
// keep the architecture note comment here
let parts = match &role.iroh {
    Some(iroh_cfg) => Some(Self::start_endpoint(EndpointArgs { config, iroh_cfg, host,
        secret_key, parent_relay_url, relay_server: &relay_server,
        relay_passports: relay_passports.clone() }).await?),
    None => None,
};
Ok(Self { relay_server, relay_passports, /* the parts' fields, or None */ })
```

`start_endpoint` already takes four arguments; with three more it is over
clippy's `too_many_arguments` (7). Pass one `EndpointArgs<'_>` struct (or
pass `role` and read `iroh_cfg` from it). Pick the one that reads better;
do not add an `allow`.

**7.4 `spawn_relay_server`.** New signature
`async fn spawn_relay_server(role: &CoordinatorRole, always: &[EndpointId],
leases: Option<LeaseStore>) -> Result<Server>`; it passes both to
`build_relay_config`.

**7.5 `build_iroh_endpoint`.** It no longer makes the key. New signature
`async fn build_iroh_endpoint(iroh_cfg, relay_url, secret_key: &SecretKey)
-> Result<Endpoint>`. `start_endpoint` keeps `secret_key` for
`registration::coordinator_did(&secret_key)` and `registration::spawn(...,
secret_key, ...)` as today.

**7.6 The info router.** In `start_endpoint`:

```rust
let state = InfoState { /* as today */, identity: host.identity_info() };
let app = info_endpoint::info_router(state).merge(passport::routes(relay_passports));
let (http_info_handle, info_addr) = Self::spawn_http_info_server(config, iroh_cfg, app).await?;
```

The passport routes are merged on every coordinator, so a relay that does
not accept passports answers `not_accepted` (`H-3`).

**7.7 Connect info.** In `spawn_http_info_server`, both serve calls need the
peer address for the limiter:

```rust
server.serve(app.into_make_service_with_connect_info::<SocketAddr>()).await ...
axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await ...
```

(Verified in the cached sources: `axum-0.8.9/src/extract/connect_info.rs:90`
and `:108` implement `Connected` for both server types;
`axum-server-0.8.0/examples/remote_address.rs:12` uses this form.)

**7.8 `src/info_endpoint.rs`.** Add `pub identity: IdentityInfo` to
`InfoState` (doc: "The host substrate's node DID and controller status.").
In `get_info`, `identity: Some(state.identity.clone())`. Update the one
`InfoState { ... }` literal in `src/info_endpoint/tests.rs:42` (any
`IdentityInfo` value) and assert it comes back in the answer.

### Step 8 — `crates/coordinator`

`crates/coordinator/Cargo.toml`: add `syneroym-passport.workspace = true`
(the getter returns `LeaseStore`, also when the `iroh` feature is off).

`crates/coordinator/src/coordinator.rs`:

```rust
pub async fn init(config: &SubstrateConfig, host: &HostIdentity) -> Result<Self> {
    info!("initializing coordinator and transport bridge");
    // Before any coordinator is built, so a passport form without [tls]
    // stops startup also for a role with no Iroh section (WebRTC only, or
    // nothing): the access value must never wait unused.
    if let Some(role) = &config.roles.coordinator {
        role.access.check_tls(config.tls.is_some())?;
        // With an Iroh section, `CoordinatorIroh::init` warns about a relay
        // that is off; warn here only when there is no Iroh section at all.
        if role.access.accepts_passports() && role.iroh.is_none() {
            warn!("[roles.coordinator] access accepts passports, but no Iroh relay runs; it \
                   has no effect");
        }
    }
    // ... CoordinatorIroh::init(config, host) ...
    // webrtc unchanged; when the iroh feature is off, `host` is unused:
    // add `#[cfg(not(feature = "iroh"))] let _ = host;`
}

/// The lease store of the Iroh relay when it accepts passports.
#[must_use]
pub fn relay_leases(&self) -> Option<LeaseStore> {
    #[cfg(feature = "iroh")]
    {
        self.iroh_coordinator.as_ref().and_then(CoordinatorIroh::relay_leases)
    }
    #[cfg(not(feature = "iroh"))]
    {
        None
    }
}
```

Build the crate once with `--no-default-features --features webrtc` to check
the cfg arms.

### Step 9 — `crates/substrate`

`Cargo.toml`: add `syneroym-passport.workspace = true` (B3 needs it too).

**9.1 `src/runtime/router.rs`.** Replace `setup_identity_and_storage` (async,
returns a tuple) with a sync loader, and take its result as input:

```rust
/// The host identity, loaded before any component starts.
pub(super) struct HostKeys {
    pub(super) host: HostIdentity,
    secret_key: [u8; 32],
}

/// Load (or make) the node key and read the controller agreement. File
/// work only, so it runs before the components that need the host DID.
pub(super) fn load_host_identity(config: &SubstrateConfig) -> anyhow::Result<HostKeys> {
    let state = identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    Ok(HostKeys { host: HostIdentity::new(&state), secret_key })
}

pub(super) async fn setup_connection_router(
    config: &SubstrateConfig,
    keys: HostKeys,
    relay_leases: Option<LeaseStore>,
) -> anyhow::Result<(...same tuple...)> {
    // (the existing comment about the verified ControllerAgreement, unchanged)
    let mut effective_config = config.clone();
    if let Some(controller) = &keys.host.verified_controller {
        effective_config.iam.admin_ucan_root = Some(controller.clone());
    }
    let config = &effective_config;
    // the unowned warning, unchanged
    setup_router(config, &keys.host.node_did, keys.secret_key, relay_leases).await ...
}
```

This is today's admin-root rule, unchanged: only where the verified
controller comes from moves. `setup_router` gains `relay_leases:
Option<LeaseStore>` and, right after `let control_plane =
route_handler_deps.control_plane.clone();`:

```rust
if let Some(leases) = relay_leases {
    control_plane.set_relay_passports(leases);
}
```

(The setter pattern of `set_endpoint_publisher`, Step 10.1.)

**9.2 `src/runtime/services.rs`.** `RuntimeServices::init(config, host:
&HostIdentity)` passes `host` to `EcosystemCoordinator::init`. Add
`#[cfg(not(feature = "coordinator"))] let _ = host;`. Add:

```rust
/// The coordinator's relay lease store, when its relay accepts passports.
pub(super) fn relay_leases(&self) -> Option<LeaseStore> {
    #[cfg(feature = "coordinator")]
    { self.coordinator.as_ref().and_then(EcosystemCoordinator::relay_leases) }
    #[cfg(not(feature = "coordinator"))]
    { None }
}
```

**9.3 `src/runtime.rs` `init`.**

```rust
let observability = ObservabilityEngine::init(&config)?;
// File work only (no network), so loading it first does not change the
// start order the comment below protects.
let keys = router::load_host_identity(&config)?;
let mut services = RuntimeServices::init(&config, &keys.host).await?;
let relay_leases = services.relay_leases();
let (connection_router, endpoint_registry, supervisor, conversation, auth_did) =
    setup_connection_router(&config, keys, relay_leases).await?;
```

Keep the long ordering comment; add one sentence that the identity is loaded
before both calls because the coordinator reports and trusts the host DID.
The tests in `crates/substrate/tests/` that call
`identity::setup_substrate_identity` themselves keep working: the call is
idempotent (it loads the existing key).

### Step 10 — `crates/control_plane`

`Cargo.toml`: add `syneroym-passport.workspace = true`.

**10.1 `src/service.rs`.** New field, after `endpoint_publisher`:

```rust
/// The lease store of this node's relay when it accepts passports. Set
/// after construction by the substrate's composition root, the same
/// two-phase way as `endpoint_publisher`: the coordinator is built before
/// this service, and most `init` call sites are tests with no relay.
pub(crate) relay_passports: OnceLock<LeaseStore>,
```

Initialise it with `OnceLock::new()` in `init`. Add the setter beside
`set_endpoint_publisher` (`service.rs:282`), with the same no-op-past-first-
call behaviour:

```rust
/// Wires in the lease store of this node's passport relay, so the
/// operator's `relay-passport-*` verbs reach the running relay. Same
/// two-phase, no-op-past-first-call wiring as `set_endpoint_publisher`.
pub fn set_relay_passports(&self, leases: LeaseStore) {
    let _ = self.relay_passports.set(leases);
}
```

Tests inside `crates/control_plane` may set the field directly
(`pub(crate)`); the unit tests of Step 10.5 do not need it at all.

**10.2 `src/service/dispatch.rs`.** In `dispatch_security_interface`, after
the `substrate/admin` gate, add two arms:

```rust
"relay-passport-deny" => {
    relay_passport::deny(self.relay_passports.get(), invocation.params).await
}
"relay-passport-leases" => relay_passport::leases(self.relay_passports.get()),
```

and `mod relay_passport;` at the top of `dispatch.rs` (file
`src/service/dispatch/relay_passport.rs`). Update the doc comment of
`dispatch_security_interface` to say the interface also holds the relay's
passport deny and lease list.

**10.3 `src/service/dispatch/relay_passport.rs` (new).**

```rust
//! The operator verbs of a relay with passport access: deny a passport and
//! list the running leases. They run in the process that owns the lease
//! store, because the store loads its index only when it opens.

use serde_json::json;
use syneroym_passport::{LeaseStore, PassportId, wire::LeaseInfo};
use syneroym_rpc::{NativeResponse, RpcError, RpcResult};
use tokio::task;
use tracing::info;

const NO_PASSPORT_RELAY: &str = "this substrate runs no relay that accepts passports";

/// Params: `(passport_id,)`. Answers how many running leases ended (a bare
/// number, the WIT `result<u64, string>`).
pub(super) async fn deny(
    store: Option<&LeaseStore>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(NO_PASSPORT_RELAY.into()))?.clone();
    let (passport_id,): (PassportId,) = serde_json::from_value(params)
        .map_err(|e| RpcError::InvalidParams(format!("relay-passport-deny: {e}")))?;
    let id = passport_id.clone();
    let ended = task::spawn_blocking(move || store.deny(&id, syneroym_passport::now_secs()))
        .await
        .map_err(|e| RpcError::InternalError(format!("the deny task failed: {e}")))?
        .map_err(|e| RpcError::InternalError(format!("{e:#}")))?;
    info!(%passport_id, ended, "relay passport denied");
    Ok(NativeResponse { payload: json!(ended) })
}

/// Answers the running leases as a list of `LeaseInfo`.
pub(super) fn leases(store: Option<&LeaseStore>) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(NO_PASSPORT_RELAY.into()))?;
    let leases: Vec<LeaseInfo> =
        store.running_leases(syneroym_passport::now_secs()).iter().map(LeaseInfo::from).collect();
    let payload = serde_json::to_value(leases).map_err(|e| RpcError::InternalError(e.to_string()))?;
    Ok(NativeResponse { payload })
}
```

Positional tuple params follow the other `security` methods
(`inject-kek` takes `(kek_hex,)`). The answers follow the WIT of Step 10.4
(the existing `security` methods answer `{"status": ...}` objects while
their WIT says `result<_, string>`; the new ones do not copy that gap).

**10.4 WIT** (`crates/wit_interfaces/wit/control-plane/control-plane.wit`,
`interface security`, lines 623-627). Every control-plane method is declared
there, so add the two methods and their record:

```wit
interface security {
    inject-kek: func(kek-hex: string) -> result<_, string>;
    rotate-kek: func(new-kek-hex: string) -> result<_, string>;
    set-secret: func(service-id: string, key: string, value: list<u8>) -> result<_, string>;

    /// One running lease of this node's passport relay.
    record relay-lease {
        /// 64 lowercase hex characters.
        passport-id: string,
        owner-did: string,
        /// The node DID admitted by this lease.
        subject: string,
        /// Unix seconds.
        lease-ends-at: u64,
        /// Unix seconds of the first registration or the last renewal.
        registered-at: u64,
    }

    /// Deny a passport on this node's relay: its leases end now, and it is
    /// refused from now on. Returns how many running leases ended. Node
    /// owner (`substrate/admin`) only.
    relay-passport-deny: func(passport-id: string) -> result<u64, string>;
    /// The running leases of this node's relay. Node owner only.
    relay-passport-leases: func() -> result<list<relay-lease>, string>;
}
```

Nothing implements the `security` interface through generated bindings
today (`git grep -n 'control_plane::security'` finds nothing), so the
change compiles with no Rust edit. Check that `bindgen!` of the
`control-plane-service` world still builds (`cargo build -p
syneroym-wit-interfaces`). The JSON field names of `LeaseInfo`
(`passport_id`, ...) stay snake case while the WIT uses kebab case: that is
the existing convention (`QueuedCallInfo`, `crates/rpc/src/proxy.rs:195`,
has no serde rename, and its WIT record `proxy-queued-call` is kebab case).

**10.5 Tests.** `src/service/dispatch/relay_passport/tests.rs` (with
`#[cfg(test)] mod tests;` in `relay_passport.rs`): with
`LeaseStore::open_in_memory(&RELAY_HOME)` and a lease granted through a real
`Registrar` (copy the shape of
`apps/roymctl/src/commands/coordinator/tests.rs:36`; if `cargo dupes`
reports it as a duplicate, move that fixture into a shared place instead):

1. `leases` lists the granted lease with its five fields.
2. `deny` answers `1`; then `leases` is empty and
   `store.is_denied(&id)` is true.
3. `deny` with a malformed id is `InvalidParams` (`-32602`).
4. Both verbs with `None` are `MethodNotFound`.

The admin gate itself is the existing first line of
`dispatch_security_interface`. Do **not** add a test to
`src/service/tests.rs`: it is in `xtask/oversized-test-files.txt` at its
current length (1,383) and cannot grow. Do not change its `for method in
[...]` array either: rustfmt would wrap it.

### Step 11 — `crates/sdk`, `apps/roymctl`, `crates/smoke-tests`

**11.1 `crates/sdk`.** `Cargo.toml`: add `syneroym-passport.workspace =
true`. In `src/client/orchestrator.rs`, beside `inject_kek`:

```rust
/// Deny a passport on the substrate's relay. Returns how many running
/// leases ended. Node owner only.
pub async fn relay_passport_deny(&self, passport_id: &PassportId) -> Result<u64> {
    let res = self
        .request("security", "relay-passport-deny", serde_json::to_value((passport_id,))?)
        .await?;
    res.result
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("relay passport deny failed: {:?}", res.result))
}

/// The running leases of the substrate's relay. Node owner only.
pub async fn relay_passport_leases(&self) -> Result<Vec<LeaseInfo>> {
    let res = self.request("security", "relay-passport-leases", serde_json::json!([])).await?;
    Ok(serde_json::from_value(res.result)?)
}
```

Check how `request` surfaces a JSON-RPC error (an `Err`, or a `result` of
`null` with an `error` field) and make both methods return the server's
message on error, like `proxy_outbox` does.

**11.2 `apps/roymctl/src/commands/coordinator.rs`.**

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum PassportCommands {
    /// (Issue: keep the doc, delete the last paragraph "A relay cannot use
    /// passports yet ...")
    Issue(IssueArgs),
    /// Deny a passport on the running relay. The relay ends its leases at
    /// once and refuses the passport from now on. A connection that is
    /// already open stays until it closes. Run as the substrate's owner
    /// (`--as <name>`). There is no undo: issue a new passport instead.
    Deny {
        /// The passport id `issue` printed (64 hex characters).
        passport_id: String,
    },
    /// List the running leases of the relay. Run as the substrate's owner
    /// (`--as <name>`).
    Leases,
}
```

Keep the `IssueArgs::from` and `--node-key` help as they are: they already
state the A10 root rule (`--from`: the controller; `--node-key`: only for a
host with no controller), which `D-B2-1` follows. Add one line to the
`Issue` doc: "A host with a controller accepts only passports signed by the
controller."

`handle` becomes async and takes what the other online verbs take:

```rust
pub async fn handle(
    command: &CoordinatorCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let CoordinatorCommands::Passport { command } = command;
    match command {
        PassportCommands::Issue(args) => handle_issue(args, substrate_opt, dir).map(|_| ()),
        PassportCommands::Deny { passport_id } => {
            let id: PassportId = passport_id.parse()?; // offline check first
            let client = online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            let ended = client.relay_passport_deny(&id).await?;
            println!("passport {id} denied; {ended} running lease(s) ended");
            Ok(())
        }
        PassportCommands::Leases => {
            let client = online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            super::print_json_result(&serde_json::to_value(client.relay_passport_leases().await?)?)
        }
    }
}

async fn online_client(...) -> anyhow::Result<SyneroymClient> {
    let substrate_did = super::get_substrate_did(substrate_opt, dir)?;
    let mut client = super::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;
    Ok(client)
}
```

Use the same `wait_for_ready` timeout and `mut` pattern as
`security::handle_kek`. In `apps/roymctl/src/commands.rs:363`, call
`coordinator::handle(&command, &api_url, substrate_opt, &dir,
run_as.as_deref(), ucan_path.as_deref()).await?`.

**11.3 roymctl tests.** `src/commands/coordinator/tests.rs` (126 lines): a
test that `Deny` with a malformed id fails with the "64 lowercase hex"
message before any connection (call `handle` with an `api_url` that nothing
listens on; the error must be the parse error). `tests/cli_args.rs` (513
lines): `coordinator passport deny --help` and `leases --help` parse and
name the `--as` hint; `deny` without an id is a usage error. Keep the
existing issue-help test; it must no longer find "cannot use passports yet"
(add a negative assertion).

**11.4 `crates/smoke-tests/src/main.rs:117`.**
`CoordinatorIroh::init(&coord_config, &HostIdentity::unowned(node_did))`
where `node_did` is `derive_did_key` of a freshly generated `Identity` (the
smoke coordinator is a throw-away in-process relay).

### Step 12 — Tests

Every test allocates ports dynamically; the coordinator tests bind
`127.0.0.1:0` as the existing ones do. Each test coordinator serves at most
10 passport requests per minute from `127.0.0.1` (the limiter), so keep each
test under that count or use a second coordinator.

**12.1 Call sites of `CoordinatorIroh::init`** (pass
`&HostIdentity::random_unowned()` unless the test needs a known host):
`tests/connection_limit.rs`, `tests/info_endpoint.rs` (4),
`tests/iroh_setup.rs`, `tests/multi_hop_relay.rs` (2),
`tests/startup_config.rs` (7), `tests/tls_rotation.rs`. Also
`EcosystemCoordinator::init` in `crates/coordinator`,
`RuntimeServices::init` in `crates/substrate/src/runtime.rs`, and
`crates/smoke-tests`. Add one helper to `tests/common/mod.rs`:

```rust
/// A coordinator on a host with a fresh node key and no controller.
pub async fn init_coordinator(config: &SubstrateConfig) -> Result<CoordinatorIroh> {
    CoordinatorIroh::init(config, &HostIdentity::random_unowned()).await
}
```

and use it at every call site that needs no known host:
`CoordinatorIroh::init(&config_c)` becomes
`common::init_coordinator(&config_c)`. **`tests/multi_hop_relay.rs` is in
`xtask/oversized-test-files.txt` at 1,110 lines** and already has
`mod common;` (line 55); its two `CoordinatorIroh::init` calls are at lines
300 and 337. With the helper those two call sites keep
their line count and need no new import, so the file does not grow. Files
that do not yet have `mod common;` add it. Run `cargo xtask
check-file-lengths` after the edit; if a listed file shrank, lower its
entry.

**12.2 `tests/common/mod.rs` and `tests/common/passport.rs` (new).** Move
`generate_self_signed_cert(cert_path: &Path, key_path: &Path, common_name:
&str)` from `tests/tls_rotation.rs` into `tests/common/mod.rs` **with its
signature unchanged**: `tls_rotation.rs` calls it twice on the same paths
with a different common name, and that rewrite is the rotation it tests.
`tls_rotation.rs` then uses `common::generate_self_signed_cert` (add
`mod common;`). The new passport helpers call it once with fixed file names
under `base` and read the PEM bytes for the reqwest client themselves. Add
`pub mod passport;` to `tests/common/mod.rs`. Helpers in
`tests/common/passport.rs`:

```rust
/// A coordinator with a relay, `[tls]` on its info server, and `access`.
pub async fn passport_coordinator(base: &Path, access: CoordinatorAccess, host: &HostIdentity)
    -> Result<(CoordinatorIroh, Client /* reqwest, trusts the cert */)>;
/// The same coordinator restarted on the same `base` (same db_dir, same cert).
pub struct Fixture { pub host_key: Identity, pub owner: Identity }
impl Fixture {
    /// A passport from the host node key to `owner`, with `limits`, living `secs`.
    pub fn passport(&self, limits: Map<String, Value>, secs: u64) -> CapabilityToken;
    /// The child token for `node`, living `secs`.
    pub fn child(&self, passport: &CapabilityToken, node: &Identity, secs: u64) -> CapabilityToken;
}
/// Challenge, sign, register. Returns the status and the parsed body.
pub async fn register(client: &Client, info: SocketAddr, host_did: &str, node: &Identity,
                      token: &CapabilityToken) -> Result<Result<RegisterResponse, (StatusCode, ErrorBody)>>;
/// An Iroh endpoint with `node`'s key and only `relay` in its relay map.
/// It accepts no connections.
pub async fn node_endpoint(node: &Identity, relay: RelayUrl) -> Result<Endpoint>;
/// Whether `ep` goes online within `wait`.
pub async fn goes_online(ep: &Endpoint, wait: Duration) -> bool;

/// The ALPN of the test protocol that `accepting_node` serves.
pub const TEST_ALPN: &[u8] = b"syneroym-test/passport-echo";
/// Like `node_endpoint`, but behind an `iroh::protocol::Router` that
/// accepts `TEST_ALPN` and answers each bidirectional stream with the bytes
/// it read (an echo). Keep the `Router` alive for as long as the node must
/// accept; `router.endpoint()` is the endpoint.
pub async fn accepting_node(node: &Identity, relay: RelayUrl) -> Result<Router>;
/// From `from`, connect to `to` (its id plus only the relay URL, so the
/// path must go through the relay), open a stream on `TEST_ALPN`, send a
/// few bytes and read them back within `wait`. True when the echo arrives.
pub async fn echo_through_relay(from: &Endpoint, to: EndpointId, relay: RelayUrl,
                                wait: Duration) -> bool;
```

An echo handler already exists: `tests/iroh_setup.rs:160` builds
`Router::builder(endpoint).accept(ALPN, Echo).spawn()`. Move its `Echo`
type (and its `ProtocolHandler` impl) into `tests/common/mod.rs` and use it
from both files, instead of writing a second one (`cargo dupes` would flag
it). Tests 7, 8 and 12 use `accepting_node` for node A and
`echo_through_relay` for the check. Both endpoints must have only the relay
in their address (no direct addresses), or the test could pass over a
direct path on loopback; build the target `EndpointAddr` with
`EndpointAddr::from_parts(id, [TransportAddr::Relay(relay)])`, as
`info_addr_bytes` in `src/coordinator.rs` does.

The node's Iroh key is `SecretKey::from_bytes(&node.to_bytes())`, so its
endpoint id is its node DID. The `db_dir` of the config is under `base`
(call `config.resolve_paths()` after setting the app dirs, as
`tls_rotation.rs` does).

**12.3 `tests/relay_passport.rs` (new; split into two files if it passes
~600 lines).** Each test uses a host with no controller, so the host node
key is the passport root, except test 11.

1. `registered_node_is_admitted_without_restart` (`H-1`, `P-4` lease
   bound): before registering, a node endpoint does not go online within 3
   s; close it; register (200, `lease_ends_at` ≤ now + 24 h and ≤ the child
   expiry); a new node endpoint goes online.
2. `max_nodes_limits_distinct_nodes` (`H-2`): passport with `max_nodes = 1`;
   node A registers; node B gets 403 `limit_reached`; node A registers again
   (renewal) and gets 200.
3. `open_relay_refuses_registration` (`H-3`): `access = "everyone"`, no
   `[tls]` needed; `POST` challenge answers 403 `not_accepted`. Same for a
   list-access relay.
4. `request_checks_come_before_signatures` (`P-7` routes): a 17 KiB body is
   413 `request_too_large`; `{}` is 400 `malformed`; an unknown challenge
   is 400 `unknown_challenge`; a wrong signature is 403 `bad_signature`.
5. `eleventh_request_in_a_minute_is_refused` (`P-7`): 10 challenges, then
   429 `rate_limited`. Use its own coordinator.
6. `leases_survive_a_restart` (`P-5`): register; `shutdown` the
   coordinator and drop it; start a new one on the same `base`; a node
   endpoint goes online with no new registration.
7. `denied_passport_is_refused_at_next_connect` (`P-6`, and `P-4` "an open
   connection is not cut"): register two nodes A and B under the same
   passport; put A's endpoint online and keep it open; deny through
   `coord.relay_leases().unwrap().deny(...)` (the same store the control
   plane gets; the control-plane path is tested in 12.6). Then: a new
   endpoint for B does not go online; registering again is 403 `denied`;
   and A's open connection stays: `/v1/info` `relay.clients` still counts
   it after 2 s, and a peer that is admitted (an endpoint with the host node
   key, always admitted) can still open a QUIC connection to A through the
   relay (`Endpoint::connect` to A's `EndpointAddr` with only the relay URL,
   with a test ALPN that A accepts). The second check is the real proof; the
   count alone could also stay up for a half-closed connection.
8. `ended_lease_is_refused_at_next_connect` (`P-4`): child token living 5
   s for two nodes A and B; register both; A's endpoint online and kept
   open; wait until the leases ended. Then a new endpoint for B does not go
   online, and A's open connection still carries a QUIC connection from
   the host-node endpoint (same check as test 7).
9. `own_endpoint_and_host_node_are_admitted` (`D-B2-3`): with
   `access = "passport"` and no leases, `common::wait_for_relay` succeeds for
   the coordinator's endpoint, and an endpoint with the host node key goes
   online.
10. `info_reports_the_host_identity` (`D-B2-13`): `/v1/info` over HTTPS
    has `identity.did == host.node_did` and `controller_status == "none"`.
11. `only_the_passport_root_may_issue` (`D-B2-1`, A10 point 4): with
    `HostIdentity { verified_controller: Some(c), .. }`, a passport from
    `c` is accepted, and one from the host node key or from any other key
    is 403 `untrusted_root`. (`HostIdentity` has public fields, so the test
    builds it directly; the coordinator never checks the agreement itself.)
12. `visitor_without_a_lease_is_refused` (`D-B2-15`): register node A and
    put it online; an endpoint with a random key (a visitor that holds no
    lease) cannot connect to A through the relay within 3 s. This pins the
    closed-group behaviour, so a later change to it is a decision, not an
    accident.

**12.4 Existing tests that change.**
- `tests/startup_config.rs::rejects_disallowed_coordinator_access_variants`:
  rewrite. Passport forms without `[tls]` fail with "[tls] must be set";
  with `[tls]` (common cert helper) they boot; the bad list id still fails
  ("is not an Iroh endpoint id"). Rename to
  `passport_access_needs_tls`, and keep the bad-id case as its own test.
- `tests/info_endpoint.rs::restricted_access_does_not_count_excluded_endpoints`:
  the coordinator's own endpoint is now admitted on its list-access relay,
  so the counts become 1 (own endpoint), 2 (allowed client), still 2 after
  the excluded client. Update the comments.
- `src/config.rs` unit tests (Step 6.4) and `src/info_endpoint/tests.rs`
  (Step 7.8).

**12.5 Unit tests** of Steps 1, 2, 5, 6, 10, 11 as listed there.

**12.6 `crates/substrate/tests/relay_passport_admin.rs` (new).** Proves the
control-plane path of `deny` and `leases` on a real substrate, through the
`roymctl` command code (`D-B1-18`, `P-8`). The test holds
`common::serial_guard()` for its whole body: it runs two live nodes, and the
harness doc (`crates/substrate/tests/common/node.rs:28`) requires the guard
for that.

1. `let _guard = common::serial_guard().await;` Boot an open relay node:
   `let open = SubstrateNode::builder().boot().await;`.
2. `let [info_port] = alloc_ports::<1>(); release_held_ports(&[info_port]);`
   (the pattern of `home_relay_registry_e2e.rs:268`), and a self-signed
   cert. The substrate tests have no cert helper; add
   `generate_self_signed_cert` to `crates/substrate/tests/common/mod.rs`
   (the same openssl call as the coordinator test helper; if `cargo dupes`
   flags the pair, accept it as two test crates, or move it to a shared
   dev-only place if one exists).
3. Make the passport node a **claimed** node, so its passport root is a
   verified controller (A10): generate `controller`, then write the node key
   and a `ControllerAgreement` into the node's base directory before boot,
   exactly as `boot_claimed` does in `substrate_ownership_e2e.rs:39-52`.
   Move that body into `crates/substrate/tests/common/` as
   `pub fn write_claim(dir: &Path, controller: &Identity)` and use it from
   both tests (no copy). Boot with `.unowned()`, `.base_path(dir)`,
   `.shared_registry(open.registry_url())`, `.shared_relay(open.relay_url())`
   (so its own endpoint and the clients use the open relay, `D-B2-15`), and a
   `.configure` hook that sets `c.tls`, `role.access =
   CoordinatorAccess::Passport` and `iroh.info_http_bind_address =
   127.0.0.1:<info_port>`.
4. Issue a passport signed by `controller`, make a child token for a fresh
   node key, and register over HTTPS to `https://127.0.0.1:<info_port>`.
5. Save `controller` as `<dir>/identities/controller.key` in a roymctl
   directory, and call the command code directly (see the note after this
   list): `roymctl::commands::coordinator::handle(&CoordinatorCommands::Passport
   { command: PassportCommands::Leases }, node.registry_url(),
   Some(node.did().to_string()), &dir, Some("controller"), None).await` is
   `Ok`, then the same with `Deny { passport_id }` is `Ok`. Check the effect
   with the SDK (`node.client_as(controller).relay_passport_leases()` is
   empty after the deny; it had one lease before). To check the printed
   output too, split `handle`'s two online arms into `pub async fn
   deny(...) -> Result<u64>` and `pub async fn leases(...) ->
   Result<Vec<LeaseInfo>>` that `handle` prints, and assert on their values.
6. A client acting as another identity gets the permission-denied error
   (`-32010`) on both verbs.
7. A node without passport access (`open`) answers `MethodNotFound`.

**This test is the first of its kind; there is no pattern to copy.** No
substrate test runs an online `roymctl` command (one that goes through
`client_for`) against a live node today. `roymctl` is a library crate so
that its command code can be linked into a substrate test
(`apps/roymctl/src/lib.rs`), and `crates/substrate` has it as a
dev-dependency, but the only current use, `gateway_session_e2e.rs:45`,
calls `session::handle`, which does not go through `client_for`. One
difference to know: `client_for` builds a `SyneroymClient` that keeps the
DHT as a backup lookup (the SDK default, `enable_dht: true`,
`crates/sdk/src/client.rs:224`), while the harness clients turn it off
(`with_registry_dht(false)`, `crates/substrate/tests/common/node.rs:490`).
This is fine while the registry lookup succeeds; both nodes share one
registry here, and `api_url` is that registry's URL. If the command is
slow or hangs, look at the DHT fallback first. Do not run the
`roymctl` binary instead: a substrate test cannot rely on another package's
binary being built.

### Step 13 — Docs and close-out of the slice

1. **Change doc** `docs/planning/changes/operator-passport/change.md`:
   - Acceptance table: fill the test column of `P-4`, `P-5`, `P-6`, `P-7`
     (add the route tests), `P-8` (deny and leases), `P-10`, `H-1`, `H-2`,
     `H-3` with the files above.
   - Tasks: tick "Coordinator: challenge and register endpoints, dynamic
     access function, strict `access` parsing". Leave "`roymctl` commands"
     open (home-relay set and status are B3); add "(issue, deny, leases
     done)".
   - "Operator root" paragraph: it already matches A10 point 4; add that
     the root is read once at startup (`D-B2-1`).
   - "Use: home relay": add a short paragraph "Who can reach a leased node"
     with the closed-group rule (`D-B2-15`).
   - Deviations: a block "Coordinator decisions (date)." with `D-B2-1` to
     `D-B2-17` from §2, and a list "Requirements on B3": (a) the substrate
     signs the challenge for the host node DID read from the passport's
     resource (`substrate:<did>/relay`), not from `/v1/info`; (b) treat
     `unknown_challenge` and `challenge_expired` the same (`D-B1-25`);
     (c) `not_accepted`, `untrusted_root`, `wrong_capability`, `denied`,
     `revoked`, `limit_reached` and `expired` will not change by retrying
     soon, while `rate_limited`, `internal` and network errors are retried
     with backoff; (d) the response types accept unknown fields; (e) **the
     registering node must use a separate passport relay node in every B3
     test**: a node is always admitted to its own relay (`D-B2-3`), so a
     test with one node that uses its own relay passes even when
     registration does nothing; (f) the clients and peers in B3's tests
     reach the leased node only if they are leased too, or over a direct
     path (`D-B2-15`).
   - **Coordinators change doc**
     (`docs/planning/changes/registry-listed-coordinators/change.md:199`):
     add after `D-A3-10` "Replaced by `D-B2-3` of the passport change doc:
     the coordinator's own endpoint is admitted under every access form."
     Do not edit the text of `D-A3-10` itself.
   - `status` stays `approved` (B3 closes the doc).
2. **Living docs** (state what is true after B2; no change-doc links):
   - `docs/system-architecture.md`: the relay access list (~line 280-285):
     the passport forms work, need `[tls]`, keep leases in
     `relay_passports.db` in `storage.db_dir`, and the always-admitted
     endpoints; remove "reserved; stops the coordinator today" and the
     sentence "Passport forms are rejected at startup ...". Config table
     (~line 359). The `/v1/info` paragraph (~line 381): `identity` is
     reported; the `relay.clients` sentence about the coordinator's own
     endpoint (now admitted under every restricted form). Add the two
     endpoints, the refusal codes and the request limit to the coordinator
     endpoints part, and one short paragraph to the security section
     (passport check order, the revocation lookup, TLS only). The `max_relay_clients`
     row of the config table (~line 370): the warning for `1` now applies to
     every access form with no parent relay (Step 6.5). State the
     closed-group rule (`D-B2-15`) next to the access list: only leased,
     listed or always-admitted endpoints reach each other through the relay,
     because a visitor connects to the relay too.
   - `docs/system-requirements-spec.md`: the Relay section — grep
     `git grep -n -i -e relay -e passport docs/system-requirements-spec.md`
     and add what an operator can now do, and the closed-group limit
     (`D-B2-15`) as a stated limit, not an Envisioned item.
   - `docs/TERMINOLOGY.md`: "passport", "child token", "lease".
   - `docs/developer-guide.md`: a short operator note: set
     `access = "passport"` and `[tls]`, issue with `roymctl coordinator
     passport issue` (signed by the controller, or the node key on a host
     with no controller), deny and list with `--as <owner>`, and the
     closed-group rule: "a passport relay is for a group whose members all
     hold leases; a peer outside the group cannot reach a member through
     it". B3 adds the substrate side.
   - `crates/substrate/config.sample.toml:173-179`: the `access` comment.
     Replace the two "reserved; stops the coordinator today" lines with what
     the forms do now, and add: "The passport forms need [tls] (the
     coordinator stops without it). Only nodes with a lease, listed nodes,
     this coordinator and this node can use the relay, also to reach each
     other." Check the other sample configs too
     (`git grep -n 'stops the coordinator today'`).
   - `docs/planning/traceability-matrix.md`: check for passport rows
     (`git grep -n -e operator-passport -e 'P-[0-9]' -e 'H-[0-9]'`).
3. **Deferred backlog** `docs/planning/deferred-backlog.md`: new rows (with
   reason, target `TBD`, and a code link):
   - "Passport roots are read at startup" (`D-B2-1`).
   - "Visitors need a lease on a passport relay" (`D-B2-15`): a peer that
     only wants to reach a leased node through its relay is refused; needs
     a relay access hook that tells home clients from visitors (iroh-relay
     0.97 has none). Link `crates/coordinator_iroh/src/config.rs`
     (`relay_access`).
   - "An unowned host cannot deny a passport" (`D-B2-6`).
   - "The passport revocation check passes when the lookup fails"
     (`D-B2-8`; link `crates/router/src/route_handler/io.rs`).
   - "The passport request limit counts the TCP peer, not the client behind
     a proxy" (`D-B2-9`).
   No new `TODO` markers are planned. If you add one, add its row under
   "Open in-code markers".
4. **`status.md`**: a B2 row (Done, the tool, the test files and counts,
   the `mise run verify` result); the last row becomes "B3, C2 to C8".
5. **`task.md`**: the status line ("S0, A1 to A4, C1, B1 and B2 are done");
   under B2 add "Done <date>. Decisions `D-B2-1` to `D-B2-17` are in the
   passport change doc."; in §1 "Facts checked", the `InfoState.substrate_id`
   bullet: add that B2 now passes the host identity into the coordinator;
   under C8 add "Reuse `HostIdentity::passport_root` (`crates/core`) for the
   accepted root; do not write a second root rule (`D-B2-16`)."

## 4. Things that are ambiguous or stale in task.md and the change docs

1. **The coordinator starts before the host identity is known (stale fact).**
   task.md says "B2 must pass the host substrate's node DID and controller
   into the info server". It does not say that `RuntimeServices::init` (which
   builds the coordinator) runs *before* `setup_connection_router` (which
   loads the identity), `crates/substrate/src/runtime.rs:143-145`. Step 9
   loads the identity first. The ordering comment there warns about
   swapping the two calls; this plan does not swap them, it only moves file
   work in front.
2. **Operator root vs. the admin root.** ADR-0015 A10 point 4 fixes the
   passport root (verified controller, else the node DID; never
   `[iam].admin_ucan_root`). `substrate/admin`, which gates `deny` and
   `leases`, also honours `[iam].admin_ucan_root`. So on a host with only
   `[iam].admin_ucan_root`, the node key issues and the admin root denies.
   This plan follows A10 (`D-B2-1`) and does not amend it; the harness
   test (12.6) therefore uses a claimed node, not the harness owner.
3. **"Decide whether the coordinator's own endpoint is admitted"**: the
   same question applies to the host node's own endpoint, and to the list
   form. task.md asks only about the passport form. Decided (owner,
   2026-10-15): all three, every restricted form (`D-B2-3`). It replaces
   `D-A3-10`.
4. **"The error set is final"** (`D-B1-15`) does not hold: `H-3` and the
   request limit need two more codes (`D-B2-4`).
5. **How `deny` and `leases` reach the running service** is not specified
   (`D-B1-18` only says "through the running service"). Decided (owner,
   2026-10-15): the control plane (`D-B2-6`).
6. **"`spawn_http_info_server` takes a ready `axum::Router`; B2 merges its
   passport routes into it."** Already true since A3 (`coordinator.rs:281`).
   B2 still has to switch both serve calls to `ConnectInfo` (Step 7.7).
7. **"Run `ucan_chain_not_revoked` ... B2 makes that function reachable"**:
   it is private in `crates/router/src/route_handler/io.rs`; the coordinator
   already depends on `syneroym-router`, so a `pub use` is enough. Which
   registry it asks is not specified; this plan uses the host's
   (`D-B2-8`). A failed lookup counts as not revoked (existing behaviour).
8. **`P-10` with the relay off**: the doc says "a service with passport
   access and no TLS config stops". This plan applies it also when
   `enable_relay = false` and when the role has no
   `[roles.coordinator.iroh]` at all (`EcosystemCoordinator::init` builds
   `CoordinatorIroh` only when `role.iroh.is_some()`,
   `crates/coordinator/src/coordinator.rs:29-31`, so the check must run
   there too) (`D-B2-2`).
9. **Which TLS**: `P-10` is about `[tls]` (the info server), not
   `[roles.coordinator.tls]` (the relay). The relay itself may stay plain
   HTTP; the passport never travels over it.
10. **"The test harness running the info server with TLS is verified by
    `tls_rotation.rs`"**: that test makes its certificate with the `openssl`
    command, so the new tests need `openssl` on the test machine too. CI
    already runs `tls_rotation.rs`, so it has it.
11. **A substrate that uses its own passport relay as its home relay** (the
    harness default, and a likely operator setup) would be refused by its own
    relay without `D-B2-3`. Not mentioned anywhere.
12. **Visitors to a passport relay are refused** (`H-1` does not say so).
    Iroh opens a client connection to a peer's home relay to send to it
    (`iroh-0.97.0/src/socket/transports/relay/actor.rs:64`), and the relay
    checks access on it like on any other client. The access function sees
    only the endpoint id. So other substrates, `roymctl` and
    `SyneroymClient` reach a leased node through its passport relay only if
    they hold a lease too. Decided (owner, 2026-10-15): a passport relay is a
    closed-group relay (`D-B2-15`), with docs, a backlog row and a test
    (12.3 test 12). Test 12.6 points the node at an open relay for this
    reason, and B3's tests must account for it (Step 13.1, (f)).
13. **`roymctl` help still says passports do not work**
    (`apps/roymctl/src/commands/coordinator.rs:35-37`): removed in Step 11.2,
    as the passport doc requires.
14. **`max_lease_secs` from config**: `PassportUse::with_max_lease_secs` says
    "a service may read it from its config". No config key is planned
    (`D-B2-14`).
15. **The architecture doc says the `relay.clients` count includes the
    coordinator's own endpoint only with `access = "everyone"`** (line 381),
    and `D-A3-10` says the same. Both change with `D-B2-3`.
16. **The control-plane WIT lists every method**, but neither task.md nor
    the passport doc mentions it for the new verbs. Step 10.4 adds them to
    `interface security`.
17. **`crates/substrate/config.sample.toml:176-177`** still calls both
    passport forms "reserved; stops the coordinator today". Step 13.2 fixes
    it.
18. **Root rule shared with C8.** task.md asks that a shared part be
    written once in its final form; A10's root rule serves both passport
    uses. It lives in `crates/core/src/host_identity.rs` (`D-B2-16`), and the
    C8 line of task.md gets a note to reuse it.

## 5. Completion steps

1. No owner check is open (answered 2026-10-15).
2. Steps 1 to 13.
3. Import clean-up pass over every edited file (types by `use`, functions
   through their parent module; no inline paths with several `::` except the
   `syneroym_passport::now_secs()` style already used in the crate).
4. `cargo dupes report` filtered to the changed files (recipe in
   `AGENTS.md`); look hardest at the new test fixtures, which are close to the
   roymctl and passport fixtures.
5. Ratchets: `xtask/oversized-test-files.txt` (multi_hop_relay.rs must not
   grow), `MAX_EXACT_DUPLICATE_PERCENT` if the gate says it went down.
6. Deferred backlog (Step 13.3).
7. `mise run verify` in the background, all gates, no `--skip`.
8. Do not commit on `main`. Commit on the feature branch only when asked.
