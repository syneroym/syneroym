# M06D slice B3 — Relay passport, substrate side: implementation plan

*Reader: the developer (or AI) who implements slice B3. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready for implementation, 2026-10-16.** Section 2 lists fifteen
questions. The owner accepted Q1 to Q12 as written on 2026-10-16. Q13 to Q15
came from a review the same day, and the owner accepted them as written too.
The steps in section 4 follow every answer. When you close out the
slice, record each answer as a `D-B3-*` decision in the passport change doc
(Step 10).

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (in the background, ~20 minutes), the
  import clean-up pass, the deferred backlog, no planning references in code
  (no "M06D", "B3", "H-7", "D-B3-4" in code, comments or test names; ADR ids
  are fine), simple English in comments and docs, the 800-line file cap, the
  100-line function cap, match arms of at most ~10 lines, the module layout
  rule (`foo.rs` beside `foo/`, never `mod.rs` under `src/`), no hardcoded
  ports in tests, and "look for an existing helper first" (`cargo dupes`).
- `task.md` in this folder: §1 rows X1 and X4, §3 slices A1, A4 (the
  `D-A4-15` note), B2 and B3.
- `docs/planning/changes/operator-passport/change.md`: "Core", "Use: home
  relay", all of "Deviations", and the list "Requirements on B3" (a) to (g).
- `docs/planning/changes/registry-listed-coordinators/change.md`: `D-A4-1`
  and `D-A4-15`.
- `docs/decisions/0015-ucan-capability-model.md`: A10.
- The code listed in §1.2.

### Terms used in this plan

- **Passport**: a `CapabilityToken` signed by the relay operator, with the
  ability `relay/home` on `substrate:<host node DID>/relay`. Its audience is
  the user's owner DID.
- **Child token**: the owner re-delegates the passport to the node key of
  their substrate. Issuer: owner DID. Audience (the **subject**): the node
  DID. Its one proof is the passport.
- **Lease**: the coordinator's record that admits the subject to the relay
  until `lease_ends_at`. At most 24 hours for the relay.
- **Passport relay node (A)**: a substrate that runs a coordinator role with
  `access = "passport"` and `[tls]`.
- **Registering node (B)**: the substrate that holds a child token, registers
  with A, and uses A's relay as its home relay.
- **Info URL**: the base URL of A's coordinator info server, for example
  `https://relay.example.org:7974`. It serves `/v1/info`,
  `/v1/home-relay/challenge` and `/v1/home-relay/register`. It is a different
  port from the relay URL (see §2, Q1).
- **Relay URL**: the Iroh relay URL that goes into
  `[parent_coordinator.iroh] url`.
- **Access end**: `min(child token expiry, passport expiry)`. No lease can
  last longer than this.

## 1. What this slice delivers

### 1.1 Criteria

| ID | Criterion (from the passport change doc) | Where this plan proves it |
| --- | --- | --- |
| H-4 | Register again when a third of the lease is left; retry with backoff; warn and set a metric at 7 days or less of passport life. | Substrate unit tests (§4 Step 9.3), with a paused clock. |
| H-5 | `roymctl` can hand a passport to a substrate and show its lease. | `apps/roymctl` unit and parse tests, and the integration test (§4 Step 9.5). |
| H-7 | The substrate registers before the endpoint first connects to the relay. | Substrate unit test `boot_registers_before_the_relay_is_inserted`, and the restart case of the integration test. |
| task.md | Integration test: restricted relay, register, expiry, restart, deny, reconnect after expiry. | `crates/substrate/tests/home_relay_passport_e2e.rs` (§4 Step 9.5). |
| task.md | Close out the passport change doc. | §4 Step 10. |

### 1.2 Code this slice builds on (read it)

- `crates/substrate/src/runtime/home_relay.rs`: `HomeRelaySource`
  (`Fixed { url }` / `Registry { .. }`), `connect_home_relay`, `run_fixed`,
  `WarnLimiter`.
- `crates/substrate/src/runtime/home_relay/resolver.rs`: the private
  `Backoff` type (line ~114), and the `ResolverIo` fake-clock test pattern in
  `resolver/tests.rs` (`now_secs` = a start value plus the paused tokio clock).
- `crates/substrate/src/runtime/home_relay/io.rs`: `Http::probe` (no
  redirects, a 64 KiB answer cap). `crates/substrate/src/runtime/home_relay/cache.rs`:
  `save` (write to `.tmp`, then rename).
- `crates/substrate/src/runtime/router.rs`: `setup_router` (line ~129 builds
  the source, line ~176 calls `connect_home_relay`, line ~147 shows the
  `set_relay_passports` wiring pattern).
- `crates/router/src/home_relay.rs`: `HomeRelay::set` (its future must never
  be dropped half way), `closed`.
- `crates/passport/src/`: `registrar.rs` (`PassportChain`, `check_chain`,
  `verify_signatures`), `issue.rs` (`child_token`), `proof.rs`
  (`sign_challenge`), `wire.rs`, `error.rs`, `passport_use.rs`.
- `crates/coordinator_iroh/src/passport.rs`: the server side of the two
  endpoints, and its status codes (`D-B2-5`).
- `crates/control_plane/src/service/dispatch.rs` (`dispatch_security_interface`)
  and `dispatch/relay_passport.rs`: how B2 added owner-only methods.
- `crates/sdk/src/client/orchestrator.rs` (`relay_passport_deny`,
  `relay_passport_leases`).
- `apps/roymctl/src/commands.rs` (`run`, `client_for`), `commands/substrate.rs`,
  `commands/coordinator.rs` (`online_client`, `handle_issue`).
- `crates/substrate/tests/common/node.rs` (`SubstrateNode`, `NodeBuilder`),
  `crates/substrate/tests/relay_passport_admin.rs` (`register_test_passport`).

### 1.3 Facts checked in the code for this plan

- The info server listens on `info_http_bind_address`, or on the relay HTTP
  port plus 10 when that is unset (`crates/coordinator_iroh/src/coordinator.rs`,
  `spawn_http_info_server`). The relay URL does not tell the substrate where
  the info server is. `CoordinatorEntry` carries a separate `info_url` for
  this reason (`crates/core/src/curated_list/coordinator.rs:31`). A `Fixed`
  source has only the relay URL.
- A passport relay always has `[tls]` (`D-B2-2`). `[tls]` secures only the
  info server. The relay has its own, separate TLS setting,
  `[roles.coordinator] tls` (comment at
  `crates/coordinator_iroh/src/coordinator.rs:124-127`; `build_relay_config`
  at `crates/coordinator_iroh/src/config.rs:123` reads `role.tls`). `resolve_relay_url` gives `https://` only when the relay
  server has an HTTPS listener. So a passport relay may serve its relay on
  `http://` or `https://`, and its info server always on `https://`.
- The substrate's Iroh endpoint trusts only the built-in WebPKI roots for a
  relay certificate (`CaRootsConfig::default`,
  `iroh-relay-0.97.0/src/tls.rs:47-50`), and nothing in the substrate sets
  other roots. So a relay with `[roles.coordinator] tls` and a private or
  self-signed certificate cannot be used as a home relay today. See Q13.
- `parent_coordinator.iroh.url` is read twice: as the home relay of the
  substrate's endpoint, and as the parent relay of the node's own
  coordinator role (`CoordinatorIroh::init`, `coordinator.rs:123`;
  `resolve_relay_url` returns the parent relay first). The coordinator
  role's endpoint has a new random key at every start, so a passport relay
  always refuses it (B2 deviations, last bullet). See Q14.
- The substrate's endpoint record has no direct addresses (they are pruned in
  `build_signed_endpoint_info`, `crates/substrate/src/runtime/publish.rs`).
  `SyneroymClient` dials the target through the target's relay with a random
  Iroh key (`crates/sdk/src/client.rs`, `connect_with_mechanisms`). So under
  `D-B2-15` (a passport relay admits only leased, listed or always-admitted
  endpoints) **no `roymctl` call can reach a node through its passport
  relay**. See Q2.
- iroh 0.97 retries a refused relay connection with a backoff of 10 ms up to
  16 s, with jitter, without limit
  (`iroh-0.97.0/src/socket/transports/relay/actor.rs:340`). So after a lease
  is granted at run time, the node reconnects within about 16 s by itself. No
  forced reconnect is needed.
- `ControlPlaneService` gets late-wired handles through `OnceLock` setters
  (`set_relay_passports`, `set_endpoint_publisher`). The `security` interface
  is gated by `substrate/admin` for every method.
- The SDK `request` has no time limit, and the router has no dispatch time
  limit. A control-plane method may wait for one registration attempt.
- `crates/substrate` does not depend on `metrics` today, and has
  `async-trait` only as a dev-dependency.
- `SubstrateNode::relay_url()` in the test harness returns
  `http://127.0.0.1:<iroh port>`. This is right for every harness node,
  including a passport relay node: the harness never sets
  `[roles.coordinator] tls`. B2's `relay_passport.rs` tests pass for the same
  reason: their relay is plain HTTP, and only the info server uses the
  self-signed certificate.
- `SubstrateNode::restart()` boots again with the same builder
  (`crates/substrate/tests/common/node.rs:498-508`), so a test cannot change
  the config across a restart today.
- The endpoint address read right after `syneroym_substrate::init` may have
  no direct addresses yet: boot does not wait for the endpoint to come
  online.

## 2. Questions for the owner (ambiguous or stale items), with recommendations

Each question names what is unclear, the recommendation, and why. The owner
accepted every recommendation on 2026-10-16. Section 4 builds on them.

**Q1 (blocking). Where does the substrate register?** The change doc proposes
`roymctl substrate home-relay set --coordinator <url> --passport <file>` and
says the passport is "used for the coordinator named by
`parent_coordinator.iroh.url`". But the registration endpoints are on the
info server, which has its own port (§1.3). The relay URL cannot give the
info URL.
*Recommendation:* `set` takes `--info-url <https url>` (required) and
`--relay-url <url>` (optional; default: the node's
`parent_coordinator.iroh.url`). The substrate stores both with the token. The
stored passport is **active** only while its relay URL equals the configured
`parent_coordinator.iroh.url`. Otherwise it is kept but not used, and
`status` says why. Rejected: deriving the info URL as "relay port + 10" (it is
only a default, and wrong behind a proxy); probing `/v1/info` for the relay
URL (the coordinator reports its bind address, which may differ from the
public URL).

**Q2 (blocking). The owner cannot reach a node through its passport relay.**
`D-B2-15` was decided after the change doc chose "hand the child token to
the substrate through the control plane". With the facts in §1.3, once a
node uses a passport relay, `roymctl` (and any `SyneroymClient`) cannot reach
it through that relay. This hits `home-relay set` (first handover), renewing
with a new passport later, and `home-relay status` (H-5).
*Recommendation:* keep the control-plane path (as decided), build nothing
new for reachability in B3, and document a bootstrap order:
1. While the node still uses a relay the owner can reach (its own relay, or
   an open relay), run `home-relay set --relay-url <passport relay> ...`.
   The passport is stored but not active (Q1).
2. Change `[parent_coordinator.iroh] url` to the passport relay and restart.
   At boot the node registers first, then inserts the relay (H-7).
After that, `status` and a new `set` need a direct path to the node. Record
this as a deferred-backlog row, next to the existing "Visitors need a lease
on a passport relay" row. The integration test reaches node B over a direct
path (requirement (f)).
Alternatives (not recommended for B3, each is a bigger change): a `roymctl`
option to dial a node at a direct address; letting an owner's `roymctl` key
hold its own lease; a file drop on the host that the running substrate
watches.

**Q3. TLS trust for the info server.** Production info servers can have a
public certificate. Private relays and every test use a self-signed or
private-CA certificate. The substrate's HTTPS client must trust it.
*Recommendation:* `set --ca-cert <pem file>` (optional). The PEM is stored
with the passport and trusted for that info URL only, on top of the normal
roots. The info URL must be `https` (the token is sensitive, P-10). No
config key. `--ca-cert` covers only the info server, not the relay itself
(see Q13).

**Q4. H-7 when registration fails at boot.** "Register before the first
connect" does not say what to do when the coordinator cannot be reached.
Waiting forever would leave the node with no relay at all.
*Recommendation:* at boot, with an active passport, try up to 3 times
(backoff 5 s, then 10 s), **and stop the boot round after 30 s in total**
(`BOOT_BUDGET`). Insert the relay after the first success, after a refusal
that will not change soon (Q7), after the 3rd failure, or when the 30 s
budget ends, whichever comes first. Keep retrying in the background. So the
relay is inserted at most ~30 s after boot. Why 30 s and not less:
requirement (g) says the client must not give up on a registration while the
coordinator may still finish it. The coordinator caps its own revocation
lookup at 15 s (`D-B2-18`), so a coordinator that keeps its own limits
answers the challenge and the register well within 30 s. The budget cuts
only a coordinator that is slower than its own limits, or one that does not
answer at all. Record this as a `D-B3-*` decision. A registration that succeeds
later costs at most ~16 s more until iroh reconnects (§1.3). An attempt that
is still running when the budget ends is dropped. This is safe: only a
dropped `HomeRelay::set` is unsafe; a dropped HTTP request only wastes one
challenge on the coordinator.

**Q5. Renewal near the access end.** "Register again when a third of the
lease is left" loops near the end: the coordinator caps the lease at the
access end, so each renewal gives a shorter lease (24 h, 8 h, 2.7 h, ...).
*Recommendation:* when `lease_ends_at >= access_ends_at`, do not schedule a
renewal: it cannot extend the lease. Also never renew sooner than 60 s after
the last attempt.

**Q6. "7 days of passport life" and "set a metric".**
*Recommendation:* measure from the access end (the child token may end
before the passport). Metric: a gauge
`substrate.home_relay.access_ends_at_secs` (Unix seconds), set whenever an
**active** passport is loaded or installed. When a `set` replaces an active
passport with an inactive one, the gauge is set to `0`, which means "no
active passport". An inactive passport never sets a real value, so an alert
never fires for a passport the node does not use. An absolute time never
goes stale; an alert computes `value - now` for `value > 0`. Warning: at boot and after each registration attempt
when 7 days or less are left. Renewals run about every 16 hours, so the
warning repeats about once or twice a day.

**Q7. Which refusal codes are retried.** Requirement (c) names 7 codes that
"will not change by retrying soon", and 3 that are retried with backoff. It
does not cover `malformed`, `bad_signature`, `not_bound`, `broken_chain`,
`invalid_chain`, `request_too_large`, or a code this build does not know.
*Recommendation:* three classes, in `syneroym_passport::wire::RetryClass`:
- `NewChallenge`: `unknown_challenge`, `challenge_expired`. Ask for a new
  challenge at once, once per attempt (requirement (b)).
- `Backoff`: `rate_limited`, `internal`, a network error, an answer that is
  not valid JSON, **and any unknown code** (a newer coordinator). Backoff
  5 s doubling to 10 minutes, with jitter.
- `Settled`: every other known code. Warn, then try again after 1 hour
  (`limit_reached` can clear when another node's lease ends; the cost is
  one request an hour). Stop completely when the local clock says the access
  end has passed.

**Q8. Which key signs the child token.** The passport audience is "the owner
DID". The control-plane call needs `substrate/admin`.
*Recommendation:* `roymctl` signs with the `--as <name>` identity. It must
be the passport audience (`child_token` already refuses otherwise, with a
clear message), and it must hold `substrate/admin` on the node (controller,
or `--ucan`). No separate `--owner` flag. The child token lives as long as
the passport (no `--expires-days`).

**Q9. Control-plane method names.** *Recommendation:* two methods on the
existing `security` interface (owner only): `home-relay-set` and
`home-relay-status`. They mirror the `roymctl` verbs. B2's
`relay-passport-*` are the operator's verbs on the other side.

**Q10. What `set` answers.** *Recommendation:* the substrate validates and
stores the token, runs **one** registration attempt (when active), and
answers the status after it. Bounded by the HTTP timeouts (10 s challenge,
30 s register), so at most about 80 s with one new-challenge retry. `roymctl`
prints the result, so the owner sees `denied` or `untrusted_root` at once.

**Q11. One passport, no "clear".** *Recommendation:* a node stores at most
one passport (it has one home relay). `set` replaces it. No `clear` verb in
B3 (backlog row). Deleting `<storage.db_dir>/home_relay_passport.json` and
restarting removes it.

**Q12. `set` on a node whose source is `Registry` or none (`D-A4-15`).**
*Recommendation:* refuse; never stop the resolver. The control plane answers
`MethodNotFound` (-32601) with a text that names `parent_coordinator.iroh.url`
(same pattern as B2's "this substrate runs no relay that accepts
passports"). B3 never calls `HomeRelay::set` from an RPC, so the cancellation
problem of `HomeRelay::set` cannot happen here.

The owner accepted Q1 to Q12 on 2026-10-16. Q13 to Q15 were added after a
review on 2026-10-16, and the owner accepted them as written the same day.

**Q13. A relay with its own TLS and a private certificate (review finding).**
The relay's TLS is `[roles.coordinator] tls`, separate from `[tls]` (§1.3).
The substrate's Iroh endpoint trusts only WebPKI roots for a relay. So a
passport relay whose relay listener uses a private or self-signed
certificate cannot be a home relay.
*Recommendation:* do not build relay-certificate trust in B3. Record a
deferred-backlog row and a `D-B3-*` decision, and say in the developer guide:
a passport relay serves its relay over plain `http://` or with a
WebPKI-trusted certificate. Plain HTTP is acceptable for the relay because
iroh encrypts all traffic end to end; relay TLS hides only metadata. The
passport itself travels only to the info server, which is always HTTPS
(`--ca-cert` covers it). Why not build it now: the trusted roots are fixed
when the endpoint is built (`Endpoint::builder().ca_roots_config`), so it
needs a config key read at boot and a change to `net_iroh::build_iroh_endpoint`.
That is separate from the passport flow.

**Q14. A node's own coordinator role on a passport relay (review finding).**
A node that runs a coordinator Iroh role and sets
`parent_coordinator.iroh.url` to a passport relay gives that relay to its
coordinator endpoint too, and the relay refuses that endpoint's random key
(§1.3).
*Recommendation:* no code change to the coordinator in B3. At boot, when the
stored passport is active and `config.roles.coordinator` has an `iroh`
section, the agent logs one warning: "the coordinator role also uses
parent_coordinator.iroh.url as its parent relay, and a passport relay
refuses its key: run the coordinator role on another node, or give it an
open parent relay". Add a deferred-backlog row and a sentence in the
developer guide's bootstrap steps. In the tests, node B runs no coordinator
role (Step 9.5), so the refused endpoint adds no noise.

**Q15. "Reconnect after expiry" (review finding).** task.md asks for an
integration test of "reconnect after expiry". The relay lease always lasts
24 hours (`max_lease_secs` is not configurable, `D-B2-14`), so a real-time
test cannot let a lease end while the token is still valid.
*Recommendation:* read it as two cases and record the reading as a `D-B3-*`
decision. (1) The token ends, so the lease ends; after a new `set` the node
connects again: the integration test (case 5). (2) The lease ends while the
token is still valid, and the node registers again and reconnects: a
paused-clock unit test (`an_ended_lease_is_renewed_while_the_token_runs`,
Step 9.3).

### 2.1 Stale or incomplete statements found (fix them in Step 10)

1. Change doc, "roymctl" list: `--coordinator <url>` (see Q1).
2. Change doc, "Open questions": "No restart and no config path" is true for
   the token, but the first switch to a passport relay needs a config change
   and a restart (Q2).
3. Change doc, criteria table: H-4, H-5, H-7 have no test file.
4. Deferred backlog rows that name B3 (lines ~279, ~281, ~282: "Degraded
   node stays degraded until restart", "`HomeRelay::set` future cancellation
   drops rollback", "`run_fixed` background task cancellation"): B3 resolves
   their B3 half by refusing on non-`Fixed` nodes and by never calling
   `HomeRelay::set` from an RPC. Update them (Step 10).
5. `docs/developer-guide.md` (~line 1817) says "Substrate-side
   configuration (`roymctl substrate home-relay set` and automatic renewal)
   is added in subsequent work". Remove that sentence in Step 10.
6. `task.md` header and `status.md` say B3 is not started. Update at the end.

## 3. Design overview

```
roymctl (owner machine)                    node B (registering substrate)                     node A (passport relay)
-----------------------                    -------------------------------                    -----------------------
home-relay set                             control plane security.home-relay-set
  child_token(owner, passport, B DID)  -->   PassportControl::set_passport
  inspect_child_token (local check)            validate -> Command::Set --> agent task
                                                                             store file (0600)
                                                                             if active: register once -------> POST challenge
                                                                                                       -------> POST register
                                               <-- status (lease_ends_at or last_error)
home-relay status                     -->   security.home-relay-status -> watch channel

boot (Fixed source):
  agent task: load file -> if active: register (<= 3 tries, <= 30 s in total) -> signal "boot round done"
  relay task: wait for the signal -> HomeRelay::set(url) loop (today's run_fixed)
  agent task: loop { renew timer | Command::Set | endpoint closed }
```

- Two tasks for a `Fixed` source, ordered by a `oneshot`. The relay task is
  today's `run_fixed`, unchanged except that it first waits for the signal.
  The agent never calls `HomeRelay::set`. The relay URL always comes from the
  config.
- The agent owns the stored file and the status. It is the only writer.
- The control plane talks to the agent through a trait object
  (`HomeRelayControl`, defined in `crates/control_plane`), the same late-wired
  `OnceLock` way as `set_relay_passports`.
- Pure logic goes into `crates/passport` (token inspection, retry classes,
  wire types), so `roymctl` and the substrate check a token the same way.

## 4. Step-by-step changes

### Step 0 — Confirm two assumptions (30 minutes, no commit)

Write a throwaway test in `crates/substrate/tests/` (delete it after):

1. Boot node A as in `relay_passport_admin.rs` (passport access; `[tls]` with
   a self-signed certificate for the info server only; no
   `[roles.coordinator] tls`, so the relay is plain HTTP). Boot node B with
   `shared_relay(A.relay_url())` (`http://127.0.0.1:<A iroh port>`) and
   `configure(|c| c.roles.coordinator = None)`. Grant B a lease by hand (B's
   node key, through `register_test_passport`-style code over HTTPS to the
   info server). Check that A's `/v1/info` `relay.clients` count goes up by
   one. This confirms that a leased substrate node connects to a passport
   relay, and that the node boots with no coordinator role.
2. Add `HomeRelay::endpoint_addr` (Step 5.5) first. After B's boot, poll it
   until it lists at least one IP address (at most 10 s), keep only the IP
   addresses, and build a `SyneroymClient` for B with
   `connect_with_mechanisms(vec![EndpointMechanism::Iroh { endpoint_addr_bytes: <that address>, relay_url: None }])`.
   Call `readyz`. This confirms the direct-path client of Step 9.4, with the
   address read the same way the helper reads it (not once, right after
   `init`).

If (1) fails: stop and report; the cause is not certificate trust (the relay
is plain HTTP). If (2) fails: report; the integration tests that need a
direct path must use the bootstrap order of Q2 instead (case 8 of Step 9.5,
with more restarts).

### Step 1 — `crates/passport`

**1.1 `src/passport_use.rs`** — add:

```rust
impl PassportUse {
    /// The host node DID named by `resource` when it is this use's selector
    /// form `substrate:<did>/<selector>`. Not checked for canonical form.
    #[must_use]
    pub fn host_of<'a>(&self, resource: &'a ResourceUri) -> Option<&'a str> {
        resource.0
            .strip_prefix("substrate:")?
            .strip_suffix(self.selector)?
            .strip_suffix('/')
            .filter(|did| !did.is_empty())
    }
}
```

Tests in `src/passport_use/tests.rs`: round trip with `resource(did)`; the
bare `substrate:<did>` gives `None`; the other use's selector gives `None`;
`substrate:/relay` gives `None`.

**1.2 New `src/chain.rs`** (`pub(crate)`) — move `PassportChain` out of
`registrar.rs` unchanged, and move the bodies of `Registrar::check_chain`,
`verify_signatures` and `wrong_capability` into a rules type that does not
need a `Registrar`:

```rust
/// The chain rules of one use at one host. `is_root` decides which issuers
/// may root a passport.
pub(crate) struct ChainRules<'a> {
    pub(crate) use_: &'a PassportUse,
    pub(crate) host_node_did: &'a str,
    pub(crate) is_root: &'a dyn Fn(&str) -> bool,
}

impl ChainRules<'_> {
    /// Root, link, exact capability, delegation, expiry, then every
    /// signature through `verify_chain`. Same order and same errors as
    /// before the move.
    pub(crate) fn check(&self, chain: &PassportChain<'_>, now_secs: u64)
        -> Result<(), RegisterError>;
    fn verify_signatures(&self, chain: &PassportChain<'_>, now_secs: u64)
        -> Result<(), RegisterError>;
    fn wrong_capability(&self) -> RegisterError;
}
```

In `registrar.rs`, `check` becomes:

```rust
let chain = PassportChain::parse(token)?;
let proven = proof.verify(&self.challenges, &self.host_node_did, &self.use_, token, now_secs)?;
binding.check(proven, chain.subject(), now_secs)?;
let is_root = |issuer: &str| self.is_root(issuer);
ChainRules { use_: &self.use_, host_node_did: &self.host_node_did, is_root: &is_root }
    .check(&chain, now_secs)?;
// rest unchanged
```

Delete `Registrar::check_chain`, `verify_signatures`, `wrong_capability`.
`src/lib.rs`: add `mod chain;`. No behaviour change: all 31 registrar tests
must pass unchanged.

**1.3 New `src/presented.rs`** (`pub use presented::{PresentedToken, inspect_child_token};` in `lib.rs`):

```rust
/// What the holder of a child token can check before it registers. It does
/// not check that the passport root is accepted: only the service knows its
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedToken {
    pub passport_id: PassportId,
    /// From the passport resource. The challenge signature names this DID,
    /// never a DID the service reports about itself.
    pub host_node_did: String,
    pub owner_did: String,
    pub subject: String,
    /// `min(child expiry, passport expiry)`.
    pub access_ends_at: u64,
}

/// The shape, link, capability, delegation, expiry and signature checks of
/// `Registrar::check`, with the passport issuer taken as the root, for the
/// child token `token` held by `subject`.
///
/// # Errors
/// `Malformed` for a bad shape or a resource that is not this use's
/// selector form, `NotBound` when the audience is not `subject`, and the
/// errors of the chain check.
pub fn inspect_child_token(
    token: &CapabilityToken,
    use_: &PassportUse,
    subject: &str,
    now_secs: u64,
) -> Result<PresentedToken, RegisterError>;
```

Pseudo-code:

```
chain = PassportChain::parse(token)?
if chain.subject() != subject -> Err(NotBound)
host = use_.host_of(&chain.passport.capabilities[0].with)
       .ok_or(Malformed("the passport resource is not substrate:<did>/<selector>"))?
ensure_canonical_did(host).map_err(|e| Malformed(format!("the host node DID: {e:#}")))?
root = chain.passport.issuer_did.as_str()
ChainRules { use_, host_node_did: host, is_root: &|i| i == root }.check(&chain, now_secs)?
Ok(PresentedToken {
    passport_id: PassportId::of(chain.passport).map_err(Malformed)?,
    host_node_did: host.to_owned(),
    owner_did: chain.passport.audience_did.clone(),
    subject: subject.to_owned(),
    access_ends_at: chain.leaf.expires_at_secs.min(chain.passport.expires_at_secs),
})
```

Tests in `src/presented/tests.rs` (use `test_support.rs`): a good child token
gives the right fields; wrong subject → `not_bound`; a passport on the bare
`substrate:<did>` → `malformed`; a mailbox passport checked as relay →
`malformed` (the selector does not match); an expired child → `expired`; a
tampered passport fact → `invalid_chain`; a child with two proofs →
`malformed`.

**1.4 `src/wire.rs`** — add (keep the file under 800 lines; it is 123 now):

```rust
/// The `code` of a client-side failure in `HomeRelayStatus::last_error`:
/// no answer, or an answer that is not a refusal body.
pub const UNREACHABLE_CODE: &str = "unreachable";

/// How a registration client treats a refusal code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// `unknown_challenge`, `challenge_expired`: ask for a new challenge now.
    NewChallenge,
    /// `rate_limited`, `internal`, `unreachable`, and any code this build
    /// does not know: try again with backoff.
    Backoff,
    /// Every other known code: trying again soon gives the same answer.
    Settled,
}

impl RetryClass {
    #[must_use]
    pub fn of(code: &str) -> Self;
}

/// The params of the control-plane method `security.home-relay-set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeRelaySetRequest {
    /// The child token for the node key, with the passport as its one proof.
    pub token: CapabilityToken,
    /// Base URL of the coordinator info server. `https` only.
    pub info_url: String,
    /// The relay the passport is for. `None`: the node's
    /// `parent_coordinator.iroh.url`.
    #[serde(default)]
    pub relay_url: Option<String>,
    /// An extra root certificate (PEM) to trust for `info_url`.
    #[serde(default)]
    pub ca_cert_pem: Option<String>,
}

/// The stored home-relay passport of a node, and its last registration.
/// Accepts unknown fields, like every response type here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeRelayStatus {
    pub relay_url: String,
    pub info_url: String,
    /// False when `relay_url` is not the node's `parent_coordinator.iroh.url`:
    /// the passport is kept but not used.
    pub active: bool,
    pub passport_id: PassportId,
    pub host_node_did: String,
    pub owner_did: String,
    pub access_ends_at: u64,
    #[serde(default)]
    pub lease_ends_at: Option<u64>,
    #[serde(default)]
    pub last_attempt_at: Option<u64>,
    /// A refusal code, or `UNREACHABLE_CODE`.
    #[serde(default)]
    pub last_error: Option<ErrorBody>,
    #[serde(default)]
    pub next_attempt_at: Option<u64>,
}
```

`RetryClass::of`: `match code { "unknown_challenge" | "challenge_expired" =>
NewChallenge, "rate_limited" | "internal" | UNREACHABLE_CODE => Backoff,
"not_accepted" | "request_too_large" | "malformed" | "bad_signature" |
"not_bound" | "untrusted_root" | "broken_chain" | "wrong_capability" |
"expired" | "invalid_chain" | "denied" | "revoked" | "limit_reached" =>
Settled, _ => Backoff }`.

Tests in `src/wire/tests.rs`: build one value of **every** `RegisterError`
variant and assert its class (so a new variant without a class fails this
test: write the match over variants with no `_` arm); an unknown code is
`Backoff`; `HomeRelayStatus` parses with an extra field; `HomeRelaySetRequest`
refuses an extra field and parses with only `token` and `info_url`.

### Step 2 — `crates/control_plane`

**2.1 New `src/home_relay.rs`**, exported from `lib.rs` as
`pub use home_relay::HomeRelayControl;`:

```rust
//! The node owner's hook into the home-relay passport of this substrate.

/// Implemented by the substrate. Set only on a node whose home relay is
/// fixed by `parent_coordinator.iroh.url`.
#[async_trait::async_trait]
pub trait HomeRelayControl: Send + Sync + fmt::Debug {
    /// Check and store the passport, register once when it is active, and
    /// answer the status after that attempt. `InvalidParams` for a request
    /// that cannot be used; `InternalError` when it cannot be stored.
    async fn set_passport(&self, request: HomeRelaySetRequest) -> RpcResult<HomeRelayStatus>;
    /// `None` when no passport is stored.
    fn status(&self) -> Option<HomeRelayStatus>;
}
```

**2.2 `src/service.rs`** — field, init and setter, beside `relay_passports`:

```rust
/// The home-relay passport control of this node, when its home relay is
/// fixed. Set after construction by the substrate's composition root.
pub(crate) home_relay: OnceLock<Arc<dyn HomeRelayControl>>,
...
home_relay: OnceLock::new(),
...
/// Wires in the home-relay passport control. No-op after the first call.
pub fn set_home_relay(&self, control: Arc<dyn HomeRelayControl>) {
    let _ = self.home_relay.set(control);
}
```

**2.3 `src/service/dispatch.rs`** — in `dispatch_security_interface`, add two
arms after `relay-passport-leases`, and update its doc comment ("...relay
passport deny/leases, and the home-relay passport of this node are node-owner
operations..."):

```rust
"home-relay-set" => home_relay::set(self.home_relay.get(), invocation.params).await,
"home-relay-status" => home_relay::status(self.home_relay.get()),
```

and `mod home_relay;` beside `mod relay_passport;`.

**2.4 New `src/service/dispatch/home_relay.rs`**:

```rust
const NO_FIXED_RELAY: &str = "this substrate has no fixed home relay: a passport needs \
    [parent_coordinator.iroh] url. A node that picks its relay from the registry, or has \
    no relay, cannot use one";

/// Params: `(HomeRelaySetRequest,)`. Answers `HomeRelayStatus`.
pub(super) async fn set(
    control: Option<&Arc<dyn HomeRelayControl>>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let control = control.ok_or_else(|| RpcError::MethodNotFound(NO_FIXED_RELAY.into()))?;
    let (request,): (HomeRelaySetRequest,) = serde_json::from_value(params)
        .map_err(|e| RpcError::InvalidParams(format!("home-relay-set: {e}")))?;
    let status = control.set_passport(request).await?;
    to_response(&status)
}

/// Answers `HomeRelayStatus`, or `null` when no passport is stored.
pub(super) fn status(control: Option<&Arc<dyn HomeRelayControl>>) -> RpcResult<NativeResponse> {
    let control = control.ok_or_else(|| RpcError::MethodNotFound(NO_FIXED_RELAY.into()))?;
    to_response(&control.status())
}
```

(`to_response`: `serde_json::to_value` mapped to `InternalError`; check
`relay_passport.rs` first and share one helper if it has the same lines.)

Tests in `src/service/dispatch/home_relay/tests.rs`, with a fake
`HomeRelayControl` (a `Mutex<Option<HomeRelayStatus>>`): no control →
`MethodNotFound` for both methods; bad params → `InvalidParams`; `set` passes
the request through and returns the fake status; `status` returns `null`
when the fake has none.

**2.5 `crates/wit_interfaces/wit/control-plane/control-plane.wit`** — in
`interface security`, after `relay-passport-leases`:

```wit
/// The params of `home-relay-set`.
record home-relay-set-request {
    /// The child token JSON (a capability token with the passport as its
    /// one proof), for this node's key.
    token: string,
    /// Base URL of the coordinator info server, https only.
    info-url: string,
    /// The relay the passport is for. None: parent_coordinator.iroh.url.
    relay-url: option<string>,
    /// An extra root certificate (PEM) to trust for info-url.
    ca-cert-pem: option<string>,
}

/// One refusal or failure of the last registration.
record home-relay-error { code: string, message: string }

/// The stored home-relay passport of this node. Not named
/// `home-relay-status`: a record and a function in one interface share one
/// name space, and the function below has that name.
record home-relay-passport {
    relay-url: string,
    info-url: string,
    active: bool,
    passport-id: string,
    host-node-did: string,
    owner-did: string,
    access-ends-at: u64,
    lease-ends-at: option<u64>,
    last-attempt-at: option<u64>,
    last-error: option<home-relay-error>,
    next-attempt-at: option<u64>,
}

/// Store a home-relay passport and register once. Node owner only.
home-relay-set: func(request: home-relay-set-request) -> result<home-relay-passport, string>;
/// The stored home-relay passport, if any. Node owner only.
home-relay-status: func() -> result<option<home-relay-passport>, string>;
```

Note: the native dispatch carries the token as a JSON object, not a string.
The WIT here only documents the method (as for B2's methods). It must still
build: `crates/wit_interfaces/src/control_plane.rs` runs `bindgen!` on this
file. Check it with `wasm-tools component wit crates/wit_interfaces/wit/control-plane`
(or `cargo build -p syneroym-wit-interfaces`) before you go on. Every type and
function name in `interface security` must be unique.

### Step 3 — `crates/sdk/src/client/orchestrator.rs`

```rust
/// Store a home-relay passport on the substrate and register once. Node
/// owner only.
pub async fn home_relay_set(&self, request: &HomeRelaySetRequest) -> Result<HomeRelayStatus> {
    let res = self.request("security", "home-relay-set", serde_json::to_value((request,))?).await?;
    Ok(serde_json::from_value(res.result)?)
}

/// The stored home-relay passport, if any. Node owner only.
pub async fn home_relay_status(&self) -> Result<Option<HomeRelayStatus>> {
    let res = self.request("security", "home-relay-status", serde_json::json!([])).await?;
    Ok(serde_json::from_value(res.result)?)
}
```

### Step 4 — `crates/substrate/Cargo.toml`

- `[dependencies]`: add `metrics.workspace = true` and
  `async-trait.workspace = true` (remove `async-trait` from
  `[dev-dependencies]`).

### Step 5 — `crates/substrate/src/runtime/home_relay.rs`

**5.1 Source.** Change the variant and add the file name:

```rust
/// The file under `storage.db_dir` that keeps the home-relay passport.
const PASSPORT_FILE: &str = "home_relay_passport.json";

pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed {
        url: RelayUrl,
        passport_path: PathBuf,
        /// This node also runs a coordinator Iroh role, which takes the same
        /// `url` as its parent relay.
        coordinator_on_parent: bool,
    },
    Registry { registry_url: String, cache_path: PathBuf },
}
```

`from_config`:

```rust
return Ok(Some(Self::Fixed {
    url,
    passport_path: config.storage.db_dir.join(PASSPORT_FILE),
    coordinator_on_parent: config.roles.coordinator.as_ref().is_some_and(|c| c.iroh.is_some()),
}));
```

Update `home_relay/tests.rs::url_is_a_fixed_source` to expect both new
fields, and add `fixed_source_notes_a_coordinator_role`.

**5.2 Shared helpers moved here** (to stay free of duplicates):

- `Backoff` from `resolver.rs` (struct, `new`, `next`, `reset`), made
  `pub(super)`. `resolver.rs` then imports `super::Backoff`. Its test
  (`resolver/tests.rs:333`) moves to `home_relay/tests.rs`.
- `pub(super) fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()>`:
  the body of `cache::save` after the encode. `cache::save` calls it. Mode
  `0600` for the cache too is harmless. Steps:
  1. Create the parent directory.
  2. Open `<path>.tmp` with `write`, `create`, `truncate` and, on Unix,
     `mode(0o600)`.
  3. On Unix, call `file.set_permissions(Permissions::from_mode(0o600))`
     **before** writing. `mode` applies only when the file is created, so a
     `.tmp` left by a crash would keep its old mode (the `D-B1-28` trap;
     `write_secret_file` in `apps/roymctl/src/commands.rs` does the same).
  4. Write the bytes, `sync_all`, rename over `path`.
  Test: a `.tmp` that already exists with mode `0644` ends as `0600`.
- `pub(super) async fn read_capped(response: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>>`:
  the chunk loop of `io::Http::probe`. `probe` calls it.

**5.3 `connect_home_relay`** — new signature and body:

```rust
/// Starts the home-relay task of `source`. Returns the passport control
/// when the source is `Fixed`; the caller wires it into the control plane.
pub(super) fn connect_home_relay(
    home_relay: HomeRelay,
    source: Option<HomeRelaySource>,
    node_did: &str,
    secret_key: [u8; 32],
) -> Option<Arc<dyn HomeRelayControl>> {
    match source {
        None => { warn!(<unchanged text>); None }
        Some(HomeRelaySource::Fixed { url, passport_path, coordinator_on_parent }) => {
            let (boot_done_tx, boot_done_rx) = oneshot::channel();
            let control = passport::start(passport::Start {
                home_relay: home_relay.clone(),
                fixed_url: url.clone(),
                path: passport_path,
                node_did: node_did.to_owned(),
                secret_key,
                coordinator_on_parent,
                boot_done: boot_done_tx,
            });
            tokio::spawn(async move {
                // The agent registers before the relay goes into the map, so
                // iroh's reconnect backoff does not delay a granted lease. A
                // dropped sender (the agent ended) also releases the wait.
                let _ = boot_done_rx.await;
                run_fixed(home_relay, url).await;
            });
            Some(control)
        }
        Some(HomeRelaySource::Registry { registry_url, cache_path }) => {
            tokio::spawn(resolver::run(home_relay, registry_url, cache_path, node_did.to_owned()));
            None
        }
    }
}
```

`run_fixed` itself does not change. Rename the parameter `chooser` to
`node_did` (it is the substrate DID; the resolver still uses it to rank).

**5.4 `mod passport;`** beside `mod cache; mod io; mod resolver;`.

**5.5 `crates/router/src/home_relay.rs`** — one read-only accessor, for the
test harness's direct-path client (Step 9.4):

```rust
/// The endpoint's current address, read live from iroh. Its direct
/// addresses appear only after the endpoint has bound and probed its
/// interfaces, so a caller that needs them polls.
#[must_use]
pub fn endpoint_addr(&self) -> EndpointAddr {
    self.inner.endpoint.addr()
}
```

The module comment says the relay URL is never read back from iroh; this
accessor is for the direct addresses only, so say that in its doc comment
too. Test in `home_relay/tests.rs`: the address id equals the endpoint id.

### Step 6 — New `crates/substrate/src/runtime/home_relay/passport.rs` and children

Layout (each file well under 800 lines; functions under 100):

```
home_relay/passport.rs           agent task, PassportControl, Start, scheduling
home_relay/passport/store.rs     StoredPassport, load, save, from_request validation
home_relay/passport/client.rs    RegistrationIo trait, LiveIo (reqwest), register_once
home_relay/passport/tests.rs     FakeIo, helpers; `mod` lines for tests/*
home_relay/passport/tests/{boot,renew,refusals,set,store}.rs
```

**6.1 Constants** (`passport.rs`):

```rust
/// Registration tries at boot before the relay is inserted anyway.
const BOOT_TRIES: usize = 3;
/// The longest the relay waits for the boot registration.
const BOOT_BUDGET: Duration = Duration::from_secs(30);
/// Retry delays after a failure that may pass.
const RETRY_INITIAL: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(600);
/// Retry delay after a refusal that will not change soon.
const SETTLED_RETRY_SECS: u64 = 3_600;
/// The soonest a renewal runs after the last attempt.
const MIN_RENEW_DELAY_SECS: u64 = 60;
/// Warn when the access end is this close.
const EXPIRY_WARNING_SECS: u64 = 7 * 24 * 3_600;
/// The gauge of the access end, in Unix seconds.
const ACCESS_END_GAUGE: &str = "substrate.home_relay.access_ends_at_secs";
```

`client.rs`:

```rust
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(10);
/// Above the coordinator's 15 s revocation lookup, so the client does not
/// give up while the coordinator still works on the request.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(30);
/// The most bytes of an answer that are read.
const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// The largest accepted `ca_cert_pem`.
pub(super) const MAX_CA_CERT_BYTES: usize = 64 * 1024;
```

**6.2 `store.rs`**

```rust
/// The file content. Holds the child token and so the passport: written
/// with mode 0600.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredPassport {
    /// Normalised with `curated_list::normalize_url`.
    pub(super) relay_url: String,
    /// Normalised, `https`.
    pub(super) info_url: String,
    pub(super) token: CapabilityToken,
    #[serde(default)]
    pub(super) ca_cert_pem: Option<String>,
}

impl StoredPassport {
    /// Checks a control-plane request. Every error is `InvalidParams` with a
    /// message the owner can act on.
    pub(super) fn from_request(
        request: HomeRelaySetRequest,
        node_did: &str,
        fixed_url: &RelayUrl,
        now_secs: u64,
    ) -> RpcResult<(Self, PresentedToken)>;

    /// Whether this passport is for the configured relay.
    pub(super) fn is_for(&self, fixed_url: &RelayUrl) -> bool;
}

/// A missing file is `None`. An unreadable or invalid file is `None` with a
/// warning; the next `set` replaces it.
pub(super) fn load(path: &Path) -> Option<StoredPassport>;
pub(super) fn save(path: &Path, stored: &StoredPassport) -> anyhow::Result<()>; // write_file_atomic
```

`from_request` pseudo-code:

```
info_url = curated_list::normalize_url(&request.info_url)        -> InvalidParams("info_url: ...")
ensure info_url starts with "https://"                          -> InvalidParams("info_url must be https: the token is sensitive")
relay_url = match request.relay_url {
    Some(raw) => net_iroh::parse_relay_url("relay_url", &raw)   -> InvalidParams
    None => fixed_url.clone(),
}
relay_url = curated_list::normalize_url(relay_url.as_str())       // one spelling for `is_for`
if let Some(pem) = &request.ca_cert_pem {
    ensure pem.len() <= MAX_CA_CERT_BYTES
    reqwest::Certificate::from_pem(pem.as_bytes())               -> InvalidParams("ca_cert_pem: ...")
}
presented = inspect_child_token(&request.token, &RELAY_HOME, node_did, now_secs)
            -> InvalidParams(format!("{}: {e}", e.code()))
Ok((StoredPassport { relay_url, info_url, token: request.token, ca_cert_pem }, presented))
```

`is_for`: `curated_list::normalize_url(fixed_url.as_str()).is_ok_and(|u| u == self.relay_url)`.

**6.3 `client.rs`**

```rust
/// Where and how to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) info_url: String,
    pub(super) ca_cert_pem: Option<String>,
}

/// Why an attempt did not give a lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Failure {
    /// The coordinator answered with a refusal body.
    Refused(ErrorBody),
    /// No answer, a timeout, or an answer that is not a refusal body.
    Unreachable(String),
}

impl Failure {
    pub(super) fn body(&self) -> ErrorBody; // Unreachable -> code UNREACHABLE_CODE
    pub(super) fn class(&self) -> RetryClass; // RetryClass::of(code)
}

/// The network half of the agent. Unit tests replace it with a fake and a
/// paused clock.
pub(super) trait RegistrationIo: Send + Sync + 'static {
    fn challenge(&self, target: &Target)
        -> impl Future<Output = Result<ChallengeResponse, Failure>> + Send;
    fn register(&self, target: &Target, request: &RegisterRequest)
        -> impl Future<Output = Result<RegisterResponse, Failure>> + Send;
    /// `HomeRelay::closed`.
    fn closed(&self) -> impl Future<Output = ()> + Send;
    /// Unix seconds now.
    fn now_secs(&self) -> u64;
}

pub(super) struct LiveIo {
    home_relay: HomeRelay,
    /// The client of the last target, rebuilt when its certificate changes.
    client: Mutex<Option<(Option<String>, reqwest::Client)>>,
}
```

`LiveIo` client: `reqwest::Client::builder().https_only(true)
.redirect(redirect::Policy::none()).timeout(REGISTER_TIMEOUT)`, plus
`add_root_certificate(Certificate::from_pem(pem))` when the target has one.
`challenge` uses `.timeout(CHALLENGE_TIMEOUT)` on the request. Both POST
`format!("{info_url}{CHALLENGE_PATH}")` / `{REGISTER_PATH}`; register sends
`serde_json::to_vec(request)` with `content-type: application/json`.
Answer handling (one helper, used by both):

```
send error                      -> Unreachable(error text)
body = read_capped(resp, MAX_ANSWER_BYTES)  (error -> Unreachable)
status 2xx  -> parse T (error -> Unreachable("the answer is not valid: ..."))
otherwise   -> parse ErrorBody -> Refused(body)
               (parse error -> Unreachable(format!("HTTP {status}")))
```

`now_secs`: `syneroym_passport::now_secs()`. `closed`: `home_relay.closed()`.

One attempt, a free function so the tests drive it through the fake:

```rust
/// Challenge, sign, register. Asks for one new challenge when the first
/// is unknown or too old (both codes mean the same to a client).
pub(super) async fn register_once<I: RegistrationIo>(
    io: &I,
    target: &Target,
    node: &Identity,
    node_did: &str,
    host_node_did: &str,
    token: &CapabilityToken,
) -> Result<RegisterResponse, Failure> {
    let mut new_challenge_left = true;
    loop {
        let answer = io.challenge(target).await?;
        let challenge = Challenge::from_hex(&answer.challenge)
            .map_err(|e| Failure::Unreachable(format!("the challenge is not valid: {e}")))?;
        let signature = sign_challenge(node, host_node_did, &RELAY_HOME, &challenge, token)
            .map_err(|e| Failure::Unreachable(format!("cannot sign the challenge: {e:#}")))?;
        let request = RegisterRequest {
            token: token.clone(),
            challenge: answer.challenge,
            signer_did: node_did.to_owned(),
            signature,
        };
        match io.register(target, &request).await {
            Err(f) if f.class() == RetryClass::NewChallenge && new_challenge_left => {
                new_challenge_left = false;
            }
            other => return other,
        }
    }
}
```

(A signing error cannot really happen with a valid key; it is reported as
`unreachable` rather than adding a third failure kind. Note it in a comment.)

**6.4 `passport.rs` — the control handle**

```rust
pub(super) struct Start {
    pub(super) home_relay: HomeRelay,
    pub(super) fixed_url: RelayUrl,
    pub(super) path: PathBuf,
    pub(super) node_did: String,
    pub(super) secret_key: [u8; 32],
    pub(super) coordinator_on_parent: bool,
    pub(super) boot_done: oneshot::Sender<()>,
}

/// Spawns the agent with `LiveIo` and returns its control handle.
pub(super) fn start(start: Start) -> Arc<dyn HomeRelayControl>;

enum Command {
    Set { stored: StoredPassport, presented: PresentedToken, reply: oneshot::Sender<RpcResult<HomeRelayStatus>> },
}

#[derive(Debug)]
struct PassportControl {
    node_did: String,
    fixed_url: RelayUrl,
    commands: mpsc::Sender<Command>,          // capacity 4
    status: watch::Receiver<Option<HomeRelayStatus>>,
    now_secs: fn() -> u64,                    // syneroym_passport::now_secs; tests may swap
}

#[async_trait::async_trait]
impl HomeRelayControl for PassportControl {
    async fn set_passport(&self, request: HomeRelaySetRequest) -> RpcResult<HomeRelayStatus> {
        let (stored, presented) =
            StoredPassport::from_request(request, &self.node_did, &self.fixed_url, (self.now_secs)())?;
        let (reply, answer) = oneshot::channel();
        self.commands.send(Command::Set { stored, presented, reply }).await
            .map_err(|_| RpcError::InternalError(STOPPED.into()))?;
        answer.await.map_err(|_| RpcError::InternalError(STOPPED.into()))?
    }
    fn status(&self) -> Option<HomeRelayStatus> { self.status.borrow().clone() }
}
```

`STOPPED = "the home-relay task has stopped (the endpoint is closed)"`.

**6.5 `passport.rs` — the agent**

```rust
struct Agent<I> {
    io: I,
    node: Identity,                 // Identity::from_bytes(&secret_key)
    node_did: String,
    fixed_url: RelayUrl,
    path: PathBuf,
    coordinator_on_parent: bool,
    current: Option<Current>,
    retry: Backoff,                 // RETRY_INITIAL .. RETRY_MAX
    status_tx: watch::Sender<Option<HomeRelayStatus>>,
    commands: mpsc::Receiver<Command>,
}

/// The stored passport and what is known about it.
struct Current {
    stored: StoredPassport,
    presented: PresentedToken,
    active: bool,
    lease_ends_at: Option<u64>,
    last_attempt_at: Option<u64>,
    last_error: Option<ErrorBody>,
    /// Unix seconds of the next attempt. `None`: no attempt is planned.
    next_attempt_at: Option<u64>,
}
```

Main loop (keep each function under 100 lines; the `select!` arms only call
methods). `Closed` is a unit struct: the endpoint closed, the task ends.

```
async fn run(mut self, boot_done: oneshot::Sender<()>):
    self.load_current()
    self.warn_if_ending(now)
    if self.current is active and not past its access end:
        if self.boot_round().await.is_err(): return       // closed during boot
    let _ = boot_done.send(())
    loop:
        let sleep = sleep_until_attempt(self.wait())       // pending forever when None
        select! {
            () = self.io.closed() => return,
            command = self.commands.recv() => match command {
                None => return,
                Some(Command::Set { stored, presented, reply }) =>
                    if self.apply_set(stored, presented, reply).await.is_err() { return },
            },
            () = sleep => if self.attempt_or_close().await.is_err() { return },
        }
```

`wait()`: `next_attempt_at.saturating_sub(io.now_secs())` as a `Duration`.
`sleep_until_attempt(None)` is `future::pending()`. `mpsc::Receiver::recv`
is cancel-safe.

**Every attempt races `io.closed()`**, so after teardown no HTTP call runs on
(review finding). Split one attempt into a read-only half and a recording
half, so both futures in the `select!` borrow `self` immutably:

```rust
/// The HTTP half: `None` when there is nothing to register (no active
/// passport). Borrows `self` immutably.
async fn registration(&self) -> Option<Result<RegisterResponse, Failure>>;

/// Records the result in `current`, plans the next attempt, warns,
/// publishes the status.
fn record(&mut self, result: Result<RegisterResponse, Failure>) -> Outcome;

/// One attempt, or `Err(Closed)` when the endpoint closes first. Dropping
/// the HTTP future is safe: it only wastes one challenge on the
/// coordinator. (Only a dropped `HomeRelay::set` is unsafe, and the agent
/// never calls it.)
async fn attempt_or_close(&mut self) -> Result<Outcome, Closed> {
    if let Some(outcome) = self.end_if_access_ended() {   // no active passport, or access ended
        return Ok(outcome);
    }
    let result = tokio::select! {
        () = self.io.closed() => return Err(Closed),
        result = self.registration() => result,
    };
    Ok(result.map_or(Outcome::Settled, |r| self.record(r)))
}
```

`Outcome` is `Leased | Settled | Retry`.

`load_current()`:

```
let Some(stored) = store::load(&self.path) else { return }
match inspect_child_token(&stored.token, &RELAY_HOME, &self.node_did, now) {
    Ok(presented) => self.install(stored, presented),
    Err(RegisterError::Expired) => {
        // Keep showing the passport in `status`. Its fields are read by
        // checking the token at its own start time; access has ended.
        let presented = inspect_child_token(&stored.token, &RELAY_HOME, &self.node_did,
                                            stored.token.not_before_secs)?   // on error: warn and return
        self.install(stored, presented)
        current.last_error = Some(<ErrorBody of RegisterError::Expired>)
        current.next_attempt_at = None
        warn!("the stored home-relay passport has ended; ask the relay operator for a new one")
        self.publish()
    }
    Err(e) => warn!("the stored home-relay passport cannot be used: {e}; run `roymctl substrate home-relay set` again"), return
}
```

`install(stored, presented)`:

```
let active = stored.is_for(&self.fixed_url)
let was_active = self.current.as_ref().is_some_and(|c| c.active)
self.current = Some(Current { stored, presented, active, lease_ends_at: None, last_attempt_at: None,
                              last_error: None, next_attempt_at: active.then_some(now) })
self.retry.reset()
// The gauge only ever names the passport in use (review finding).
if active { gauge!(ACCESS_END_GAUGE).set(access_ends_at as f64) }
else if was_active { gauge!(ACCESS_END_GAUGE).set(0.0) }        // 0: no active passport
if !active {
    info!("the home-relay passport is for <relay_url>, but parent_coordinator.iroh.url is \
           <fixed_url>: it is kept and not used. Set the url to <relay_url> and restart to use it")
} else if self.coordinator_on_parent {
    warn!("the coordinator role also uses parent_coordinator.iroh.url as its parent relay, and a \
           passport relay refuses its key: run the coordinator role on another node, or give it \
           an open parent relay")
}
self.publish()
```

`boot_round()` (H-7, Q4): at most `BOOT_TRIES` attempts **and** at most
`BOOT_BUDGET` in total.

```rust
/// Registration before the relay goes into the map. Ends after a lease, a
/// settled refusal, `BOOT_TRIES` failures or `BOOT_BUDGET`, whichever comes
/// first. `Err(Closed)` when the endpoint closes.
async fn boot_round(&mut self) -> Result<(), Closed> {
    let budget = time::sleep(BOOT_BUDGET);
    tokio::pin!(budget);
    for try_no in 1..=BOOT_TRIES {
        let outcome = tokio::select! {
            () = &mut budget => { self.boot_budget_spent(); return Ok(()) }
            outcome = self.attempt_or_close() => outcome?,
        };
        if outcome != Outcome::Retry || try_no == BOOT_TRIES {
            return Ok(());
        }
        tokio::select! {
            () = &mut budget => return Ok(()),
            () = self.io.closed() => return Err(Closed),
            () = time::sleep(self.wait()) => {}
        }
    }
    Ok(())
}
```

`boot_budget_spent()`: `last_error = Some(ErrorBody { code: UNREACHABLE_CODE,
message: "no answer within 30 s at boot" })`, `last_attempt_at = Some(now)`,
`next_attempt_at = Some(now + retry.next())`, publish.

A `home-relay-set` call that arrives during the boot round waits in the
channel until the round ends (at most `BOOT_BUDGET`), then runs. This is
accepted: it happens only in the first ~30 s after boot.

`record(result)`:

```
let now = io.now_secs()                          // the attempt may take seconds
current.last_attempt_at = Some(now)
let outcome = match result {
    Ok(resp) => self.on_lease(resp, now),        // Leased
    Err(f) => self.on_failure(f, now),           // Settled or Retry
};
self.warn_if_ending(now)
self.publish()
outcome
```

`end_if_access_ended()`: when there is no active passport, or
`now >= current.presented.access_ends_at`, it **always** returns
`Some(Outcome::Settled)`, so an ended passport never makes a request. Only
the first time it sees the ended access (`next_attempt_at` is not yet
`None`) does it also set `next_attempt_at = None`, warn "home-relay passport
access has ended ...", and publish. Otherwise it returns `None`. Unit test:
`an_ended_passport_never_requests_again` (call `attempt_or_close` twice after
the end; no IO call, one warning).

`on_lease(resp, now)`:

```
self.retry.reset()
if resp.passport_id != presented.passport_id: warn!(...)   // a coordinator bug; keep going
current.lease_ends_at = Some(resp.lease_ends_at); current.last_error = None
current.next_attempt_at = renew_at(now, resp.lease_ends_at, presented.access_ends_at)
info!(lease_ends_at, "home-relay lease granted")
```

```rust
/// A third of the lease left, never sooner than `MIN_RENEW_DELAY_SECS`.
/// `None` when the lease already reaches the access end: a renewal cannot
/// extend it.
fn renew_at(now: u64, lease_ends_at: u64, access_ends_at: u64) -> Option<u64> {
    if lease_ends_at >= access_ends_at {
        return None;
    }
    let third = lease_ends_at.saturating_sub(now) / 3;
    Some(lease_ends_at.saturating_sub(third).max(now + MIN_RENEW_DELAY_SECS))
}
```

`on_failure(f, now)`:

```
let body = f.body()
// A deny deletes every lease of the passport on the coordinator, so the
// last lease no longer runs. Every other failure keeps it: the coordinator
// still admits the node until it ends (a revocation does not end a running
// lease either).
if body.code == RegisterError::Denied.code() { current.lease_ends_at = None }
current.last_error = Some(body)
match f.class() {
    Settled => { warn!(code, message, "the coordinator refused the home-relay passport; trying again in an hour");
                 next = now + SETTLED_RETRY_SECS }
    NewChallenge | Backoff => { debug!/warn! through a WarnLimiter; next = now + retry.next().as_secs().max(1) }
}
// Never plan past the access end.
current.next_attempt_at = Some(next).filter(|t| *t < presented.access_ends_at)
```

`warn_if_ending(now)` (H-4, Q6), only for an active passport:

```rust
/// True when the access end is `EXPIRY_WARNING_SECS` away or closer.
fn ending_soon(now: u64, access_ends_at: u64) -> bool {
    access_ends_at.saturating_sub(now) <= EXPIRY_WARNING_SECS
}
```

When true: `warn!(access_ends_at, days_left, "the home-relay passport ends soon; ask the relay operator for a new one and run `roymctl substrate home-relay set`")`.

`apply_set(stored, presented, reply) -> Result<(), Closed>`:

```
match spawn_blocking(store::save(path, &stored)) {
    Err/Ok(Err(e)) => { let _ = reply.send(Err(RpcError::InternalError(format!("cannot store the passport: {e:#}")))); return Ok(()) }
    Ok(Ok(())) => {}
}
self.install(stored, presented)                  // replaces; resets retry and status
if active:
    if self.attempt_or_close().await.is_err() {
        return Err(Closed)                       // `reply` is dropped: the caller gets STOPPED
    }
let _ = reply.send(Ok(self.status_now()))
Ok(())
```

`status_now()` / `publish()`: build `HomeRelayStatus` from `current` (all
fields of §Step 1.4). `publish` does `status_tx.send_replace(...)`.

`start(start)`: build `Agent` with `LiveIo::new(home_relay)`, the channels,
and `tokio::spawn(agent.run(boot_done))`. Return `Arc::new(PassportControl { .. })`.

### Step 7 — `crates/substrate/src/runtime/router.rs`

In `setup_router`, replace lines ~176-178:

```rust
if let Some(handle) = router.home_relay() {
    let passport = home_relay::connect_home_relay(handle, relay_source, service_id, secret_key);
    if let (Some(control_plane), Some(passport)) = (&control_plane, passport) {
        control_plane.set_home_relay(passport);
    }
}
```

`control_plane` is the `Option<Arc<ControlPlaneService>>` already cloned at
line ~146. No other caller of `connect_home_relay` exists.

### Step 8 — `apps/roymctl`

**8.1 Shared client.** Move `online_client` from `commands/coordinator.rs`
to `commands.rs` as `pub(crate) async fn online_client(...)`. `coordinator.rs`
calls `super::online_client`.

**8.2 `commands/substrate.rs`**: add the variant and the module, and widen
`handle`:

```rust
pub mod home_relay;

/// Give this substrate a passport for a private home relay, or show it.
HomeRelay {
    #[command(subcommand)]
    command: home_relay::HomeRelayCommands,
},

pub async fn handle(
    command: &SubstrateCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()>
// new arm:
SubstrateCommands::HomeRelay { command } => {
    home_relay::handle(command, api_url, substrate_opt, dir, run_as, ucan_path).await?;
}
```

Call site: `commands.rs` `run`, `Commands::Substrate { command } =>
substrate::handle(&command, &api_url, substrate_opt, &dir, run_as, ucan).await?`.
(`substrate_opt` is moved here; it is not used after that arm.)

**8.3 New `commands/substrate/home_relay.rs`**:

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum HomeRelayCommands {
    /// Hand a relay passport to the running substrate. The command signs a
    /// child token of the passport for the substrate's node key with the
    /// `--as` identity, which must be the passport's owner and own the
    /// substrate. The substrate registers at once and renews by itself.
    /// A substrate whose home relay is a passport relay can only be reached
    /// by a direct path: run this before you point
    /// `[parent_coordinator.iroh] url` at the passport relay, then restart.
    Set(SetArgs),
    /// Show the stored passport, the lease and the last error.
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct SetArgs {
    /// Base URL of the coordinator's info server, `https://host:port` (the
    /// address of `/v1/info`). It is not the relay URL.
    #[arg(long)]
    pub info_url: String,
    /// The passport file from `roymctl coordinator passport issue`.
    #[arg(long)]
    pub passport: PathBuf,
    /// The relay the passport is for. Defaults to the substrate's
    /// `parent_coordinator.iroh.url`.
    #[arg(long)]
    pub relay_url: Option<String>,
    /// A PEM root certificate to trust for the info server.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,
}

/// Offline part of `set`: read the files, sign the child token, check it.
/// Public so tests can send the request over another connection.
pub fn build_set_request(
    args: &SetArgs,
    node_did: &str,
    dir: &Path,
    run_as: Option<&str>,
) -> anyhow::Result<HomeRelaySetRequest>;

pub async fn set(args: &SetArgs, api_url: &str, substrate_opt: Option<String>, dir: &Path,
    run_as: Option<&str>, ucan_path: Option<&Path>) -> anyhow::Result<HomeRelayStatus>;

pub async fn status(api_url: &str, substrate_opt: Option<String>, dir: &Path,
    run_as: Option<&str>, ucan_path: Option<&Path>) -> anyhow::Result<Option<HomeRelayStatus>>;

pub async fn handle(command: &HomeRelayCommands, api_url: &str, substrate_opt: Option<String>,
    dir: &Path, run_as: Option<&str>, ucan_path: Option<&Path>) -> anyhow::Result<()>;
```

`build_set_request` pseudo-code:

```
let name = run_as.context("home-relay set signs with the passport owner's key: pass --as <name>")?
let owner = Identity::load_from_path(identity::require_key_present(dir, name, "Identity")?)?
let passport: CapabilityToken = serde_json::from_str(&fs::read_to_string(&args.passport)
        .with_context(|| format!("cannot read {}", args.passport.display()))?)
        .context("the passport file is not a passport")?
let now = syneroym_passport::now_secs()
let life = passport.expires_at_secs.saturating_sub(now)
let child = syneroym_passport::child_token(&owner, &passport, node_did, life, now)?   // clear errors: wrong owner, expired
syneroym_passport::inspect_child_token(&child, &RELAY_HOME, node_did, now)
        .map_err(|e| anyhow!("{}: {e}", e.code()))?
let ca_cert_pem = args.ca_cert.as_ref().map(fs::read_to_string).transpose()?
Ok(HomeRelaySetRequest { token: child, info_url: args.info_url.clone(), relay_url: args.relay_url.clone(), ca_cert_pem })
```

`set`: `node_did = super::super::get_substrate_did(substrate_opt.clone(), dir)?`,
`request = build_set_request(...)`, `client = super::super::online_client(...)`,
`client.home_relay_set(&request).await`.

`handle` prints `print_json_result` of the status, then the line from a
pure function, so the rule order is fixed and tested:

```rust
/// The one-line summary under the status JSON. `now` decides whether the
/// last lease still runs.
pub fn summary_line(status: Option<&HomeRelayStatus>, now: u64) -> String
```

Rules, first match wins:
1. `None` → "no home-relay passport is stored".
2. `!active` → "stored, not used: parent_coordinator.iroh.url is not <relay_url>".
3. lease running (`lease_ends_at > now`) and `last_error` → "the lease runs
   until <t>, but the last registration failed: <code>: <message>".
4. lease running → "registered; the lease ends at <t>".
5. `last_error` → "not registered: <code>: <message>".
6. otherwise → "not registered yet; next attempt at <t>" (or "no attempt
   planned" when `next_attempt_at` is `None`).

### Step 9 — Tests

**9.1 Unit tests in `crates/passport`** — Step 1 lists them.

**9.2 Unit tests in `crates/control_plane`** — Step 2.4.

**9.3 Substrate agent unit tests** — `home_relay/passport/tests.rs` holds a
`FakeIo` modelled on the resolver's: scripted answers per call
(`VecDeque<Result<..., Failure>>` for challenge and register), a record of
calls with their `now_secs`, `now_secs = START_SECS + started.elapsed()` on
the paused tokio clock, and a `CancellationToken` for `closed`. Real keys
from `syneroym_passport::issue_passport` / `child_token`. A helper that runs
the agent with a `oneshot` and returns the control handle, the status
receiver, and a flag set when `boot_done` fires.

All `#[tokio::test(start_paused = true)]`. Test names say the behaviour:

`tests/boot.rs`
- `no_stored_passport_releases_the_relay_at_once` (no IO calls).
- `boot_registers_before_the_relay_is_inserted` (H-7): the register call is
  recorded before `boot_done` fires.
- `boot_inserts_the_relay_after_three_failed_tries`: three `Unreachable`;
  `boot_done` fires after the third; a 4th attempt follows later.
- `boot_inserts_the_relay_after_a_settled_refusal`: one `denied`, then
  `boot_done` at once.
- `inactive_passport_is_kept_and_not_used`: stored `relay_url` differs; no IO;
  status `active == false`.
- `expired_stored_passport_makes_no_attempt` (status still shows it, with
  `last_error.code == "expired"`).
- `unreadable_file_is_ignored` (garbage JSON → no status, no panic).
- `boot_round_ends_after_its_budget`: the fake's challenge never answers
  (`future::pending`); `boot_done` fires at 30 s; status
  `last_error.code == "unreachable"`; a later attempt is planned.
- `closing_the_endpoint_stops_a_running_attempt`: the fake register never
  answers; cancel `closed`; the agent task ends (join it with a timeout) and
  makes no further call.
- `a_set_during_boot_runs_after_the_round`: a `set` sent while the boot
  round waits is answered after `boot_done`.
- `an_active_passport_with_a_coordinator_role_warns`. Use the log-capture
  helper of `home_relay/resolver/tests/logs.rs` (`MockWriter`, `capture`).
  It is private to the resolver tests today, so **move** it into a shared
  test module, `crates/substrate/src/runtime/home_relay/test_logs.rs`
  (declared `#[cfg(test)] mod test_logs;` in `home_relay.rs`, items
  `pub(super)`), and use it from both test trees. Do not copy it:
  `cargo dupes` would flag the copy. Do not add `tracing-test`.

`tests/renew.rs` (H-4)
- `renews_when_a_third_of_the_lease_is_left`: lease of 24 h → next attempt at
  +16 h; advance 16 h; second register recorded.
- `an_ended_lease_is_renewed_while_the_token_runs` (Q15 case 2): a lease of
  24 h; renewals fail with `Unreachable` until after the lease end; the next
  success gives a new lease and the status shows it.
- `no_renewal_when_the_lease_reaches_the_access_end` (Q5).
- `renew_at_never_sooner_than_a_minute` (pure `renew_at` cases).
- `ending_soon_is_true_at_seven_days` (pure `ending_soon` boundary: 7 d → true,
  7 d + 1 s → false).
- `access_end_gauge_names_only_the_active_passport`: an active passport sets
  the access end; an inactive one at boot sets nothing; a `set` that
  replaces an active passport with an inactive one sets `0`. Install a
  `metrics_util::debugging::DebuggingRecorder` locally if the workspace
  already has `metrics-util`; otherwise test through a small
  `report_access_end(f64)` seam. Check `crates/observability` for an existing
  test recorder before adding a crate.

`tests/refusals.rs` (Q7)
- `a_stale_challenge_is_retried_once_at_once`: `unknown_challenge`, then
  success, in one attempt; `challenge_expired` twice → failure recorded.
- `transient_failures_back_off`: `rate_limited`, `internal`, `Unreachable`, an
  unknown code: attempts at about +5 s, +10 s, +20 s (±10 % jitter: assert
  ranges).
- `settled_refusal_waits_an_hour`: each settled code waits 3,600 s.
- `no_attempt_after_the_access_end`.
- `failure_keeps_the_last_lease_in_the_status` (`rate_limited`, `revoked`).
- `denied_clears_the_lease_in_the_status`.

`tests/set.rs`
- `set_stores_registers_once_and_answers_the_status`.
- `set_replaces_the_old_passport_and_resets_the_backoff`.
- `set_for_another_relay_is_stored_inactive`.
- `set_answers_internal_error_when_the_file_cannot_be_written` (path in a
  read-only or missing-parent location that cannot be created, e.g. a path
  under a regular file).

`tests/store.rs`
- `save_then_load_round_trips`; `saved_file_has_mode_0600` (`#[cfg(unix)]`);
  `a_left_over_tmp_file_gets_mode_0600` (create `<path>.tmp` with `0644`
  first; `#[cfg(unix)]`). Put the last one beside `write_file_atomic` in
  `home_relay/tests.rs` if it tests the helper alone.
- `from_request_refuses` table: `http://` info URL, query in the info URL, bad
  relay URL, a CA that is not PEM, a CA over 64 KiB, a token for another
  node, an expired token, a token for the mailbox use. Each error is
  `InvalidParams` and names the field.
- `relay_url_defaults_to_the_fixed_url`; `is_for_ignores_a_trailing_slash`.

`client.rs` HTTP mapping: a small test in `passport/tests/store.rs` or its
own `tests/client.rs` against a local axum server on `127.0.0.1:0` with TLS
is heavy; instead test the answer-mapping helper with
`http::Response` values converted into `reqwest::Response`
(`reqwest::Response::from(http::Response<_>)`): 200 + body → Ok; 403 +
`ErrorBody` → `Refused`; 502 + HTML → `Unreachable("HTTP 502 ...")`; body over
64 KiB → `Unreachable`.

**9.4 Test harness — `crates/substrate/tests/common/`**

- `node.rs`: no change to `relay_url()`. It stays `http://`: the harness
  never sets `[roles.coordinator] tls`, so every harness relay, a passport
  relay included, is plain HTTP (§1.3).
- `node.rs`: add a direct-path client. It reads the address **live** through
  `HomeRelay::endpoint_addr` (Step 5.5), not once right after `init`, because
  the direct addresses appear only after the endpoint has bound:

  ```rust
  /// A client for this node over a direct path (no relay), acting as
  /// `caller`. For a node whose home relay refuses visitors. Waits up to
  /// 10 s for the endpoint to list a direct address.
  pub async fn direct_client_as(&self, caller: Identity) -> SyneroymClient
  ```

  Pseudo-code: poll `self.home_relay().endpoint_addr()` every 100 ms until it
  has at least one IP address (panic after 10 s with the last address); keep
  only its IP addresses (`EndpointAddr::from_parts(id, ip addrs as
  TransportAddr::Ip)`); build `SyneroymClient::new_with_identity(did,
  registry_url, caller).with_registry_dht(false)` and call
  `connect_with_mechanisms(vec![EndpointMechanism::Iroh {
  endpoint_addr_bytes: serde_json::to_vec(&addr), relay_url: None }])`.
- `node.rs`: add a restart that changes the config, for the bootstrap test
  (review finding):

  ```rust
  /// Like `restart`, but boots with `change(builder)`. Keeps the directory,
  /// the identity and the ports.
  pub async fn restart_with(self, change: impl FnOnce(NodeBuilder) -> NodeBuilder) -> SubstrateNode
  ```

  Pseudo-code: the body of `restart`, with `change(self.builder.clone())` in
  place of `self.builder.clone()`. `restart` becomes
  `self.restart_with(|b| b).await`.
- New `common/passport.rs` (`pub mod passport;` in `common/mod.rs`): move
  `setup_certs` and `register_test_passport` out of
  `relay_passport_admin.rs` (that test then imports them), and add:
  - `pub struct PassportRelay { pub open: SubstrateNode, pub relay: SubstrateNode, pub controller: Identity, pub cert_pem: String, pub info_url: String }`
    and `pub async fn boot() -> Self` (the setup of `relay_passport_admin.rs`:
    an open node; a claimed node with `[tls]` and `access = Passport`, with
    `shared_registry` and `shared_relay` of the open node, and an
    `alloc_ports` info port). `pub fn relay_url(&self) -> &str` (the relay
    node's `http://` relay URL). `pub async fn restart_relay(&mut self)`.
  - `pub fn issue(&self, owner_did: &str, secs: u64) -> CapabilityToken` (from
    the controller).
  - `pub async fn leased_peer(&self, passport: &CapabilityToken, owner: &Identity) -> Endpoint`:
    a raw iroh endpoint with a fresh key, only `relay_url()` in its map, no IP
    transports, registered with its own child token (challenge, sign,
    register).
  - `pub async fn reaches(peer: &Endpoint, node: &SubstrateNode, relay: RelayUrl, wait: Duration) -> bool`:
    polls `peer.connect(EndpointAddr::from_parts(node_id, [TransportAddr::Relay(relay)]), SYNEROYM_ALPN)`
    until it succeeds or `wait` ends. A connection proves node B is connected
    to the relay. Close the connection after.
  - `pub fn owner_dir(owner: &Identity) -> TempDir`: a roymctl `--dir` with
    `identities/owner.key`.

All ports through `alloc_ports`. No literals.

**9.5 Integration test — new `crates/substrate/tests/home_relay_passport_e2e.rs`**

Setup per test: `let _guard = common::serial_guard().await;`, install the
ring provider, `PassportRelay::boot()`, an `owner` identity, and node B:

```rust
SubstrateNode::builder()
    .owner(&owner)
    .shared_registry(relay.open.registry_url())
    .shared_relay(relay.relay_url())
    .configure(|c| c.roles.coordinator = None)   // see below
    .skip_ready_wait()          // the owner client cannot reach B through the relay
    .boot().await
```

Requirement (e) holds: B uses A's relay, never its own. B runs **no
coordinator role**. If it ran one, its coordinator endpoint would take the
same `url` as its parent relay, and A would refuse that random key (Q14).
That does not break the test, but it adds refused connections and log noise,
and it is the setup the developer guide tells users to avoid. Each test
function stays under 100 lines; share steps as helpers in the test file.

1. `a_node_with_a_passport_is_admitted_without_restart` (H-1 from the node,
   H-5 set and status):
   - `peer = relay.leased_peer(..)`; assert `!reaches(peer, B, 5 s)`.
   - `request = roymctl::commands::substrate::home_relay::build_set_request(
     &SetArgs { info_url, passport: <file>, relay_url: None, ca_cert: Some(<file>) },
     B.did(), owner_dir, Some("owner"))`.
   - `status = B.direct_client_as(owner).home_relay_set(&request)`; assert
     `active`, `lease_ends_at.is_some()`, `last_error.is_none()`.
   - A's `relay_passport_leases` (controller client) lists B's DID.
   - assert `reaches(peer, B, 30 s)` (iroh reconnect backoff ≤ 16 s).
   - `home_relay_status` over the direct client equals the lease.
2. `a_restarted_node_registers_before_it_connects` (H-7, restart): after the
   steps of 1, `B.restart()`; `status.last_attempt_at >= restart time` and
   `lease_ends_at` is newer; `reaches(peer, B, 30 s)`.
3. `a_restarted_relay_keeps_the_lease` (P-5 from the node side): after 1,
   `relay.restart_relay()`; a fresh `leased_peer`; `reaches(peer, B, 30 s)`.
4. `a_denied_passport_is_refused_on_the_next_connect` (deny):
   after 1, deny B's passport through A's control plane (controller); restart
   the relay (forces B to reconnect); `!reaches(peer2, B, 10 s)` where `peer2`
   holds a lease from **another** passport; a new `home_relay_set` with the
   denied passport answers `last_error.code == "denied"`.
5. `an_expired_token_ends_access_and_a_new_passport_restores_it` (expiry,
   reconnect after expiry): set with a child token that lives 20 s (build it
   in the test with `child_token(.., 20, now)`); assert registered and
   `next_attempt_at.is_none()` (Q5); sleep until it ends; restart the relay;
   `!reaches(peer, B, 10 s)`; status `lease_ends_at` is in the past; set a
   new full-life passport; `reaches(peer, B, 30 s)`.
6. `a_node_without_a_fixed_relay_refuses_set` (Q12): node C with
   `configure(|c| c.parent_coordinator.iroh.url = None)` (registry source)
   and `.skip_ready_wait()`; `C.direct_client_as(owner).home_relay_set(..)`
   fails with code -32601; message names `parent_coordinator.iroh.url`.
7. `only_the_owner_may_set_or_read` : a stranger's direct client gets
   `PERMISSION_DENIED_CODE` on both methods.
8. `the_bootstrap_order_moves_a_node_onto_a_passport_relay` (Q1/Q2, the
   production flow, and the roymctl code path of H-5). Node D starts on the
   **open** relay, owned by `owner`, with no coordinator role, and with the
   normal ready wait (the owner can reach it):
   - Call `roymctl::commands::substrate::home_relay::set(&SetArgs { info_url,
     passport, relay_url: Some(relay.relay_url()), ca_cert: Some(..) },
     D.registry_url(), Some(D.did().to_string()), owner_dir, Some("owner"), None)`. This
     runs `online_client` and `wait_for_ready` against a live node, like B2's
     test calls `coordinator::leases`.
   - Assert the answer: `active == false`, no lease; A lists no lease for D.
   - `roymctl::commands::substrate::home_relay::status(..)` gives the same
     status. `summary_line(Some(&status), now)` starts with "stored, not
     used".
   - `D = D.restart_with(|b| b.shared_relay(relay.relay_url()).skip_ready_wait()).await`.
   - Over `D.direct_client_as(owner)`: `home_relay_status` is `active`, with a
     lease and `last_attempt_at >= restart time` (registered at boot). A
     lists D's DID. `reaches(peer, D, 30 s)`.

Teardown: `teardown()` every node at the end of each test.

**9.6 `apps/roymctl`**
- `tests/cli_args.rs`: `roymctl substrate home-relay set --info-url
  https://r.example:7974 --passport p.json` parses (and with `--relay-url`,
  `--ca-cert`); `--info-url` missing fails; `roymctl substrate home-relay
  status` parses.
- New `src/commands/substrate/home_relay/tests.rs`: `build_set_request`
  without `--as` errors with "--as"; a passport for another owner errors with
  "issued to"; an expired passport errors; a good one gives a request whose
  token `inspect_child_token` accepts for the node DID; `--ca-cert` content is
  carried.
- Same file: `summary_line` for each of its six rules, including a running
  lease together with an error (rule 3) and an ended lease with an error
  (rule 5).

### Step 10 — Docs and close-out

- `docs/planning/changes/operator-passport/change.md`:
  - roymctl list: the final `home-relay set` flags (`--info-url`,
    `--passport`, `--relay-url`, `--ca-cert`) and the bootstrap order.
  - criteria table: H-4 → `crates/substrate/src/runtime/home_relay/passport/tests/renew.rs`,
    `tests/refusals.rs`; H-5 → `apps/roymctl/src/commands/substrate/home_relay/tests.rs`,
    `apps/roymctl/tests/cli_args.rs`, `crates/substrate/tests/home_relay_passport_e2e.rs`;
    H-7 → `tests/boot.rs`, `home_relay_passport_e2e.rs`.
  - Tasks: tick the four open boxes.
  - Deviations: a new block "Substrate decisions (date)" with `D-B3-1` …
    for every answer of §2 as the owner confirmed it, plus any code-vs-plan
    difference found while coding.
  - Front matter: `status: done`, and `living-docs-touched:` listing every
    living doc **the whole change** (B1 to B3) touched, not only B3's. It is
    empty today. B2 already edited `docs/TERMINOLOGY.md` (Passport, Child
    token, Lease), `docs/developer-guide.md`, `docs/system-architecture.md`
    and `docs/system-requirements-spec.md`; B3 adds the files below,
    including `docs/planning/traceability-matrix.md`. Check with
    `git diff --stat main...HEAD -- docs` and the B1 commit (`21e3ff86`).
- Living docs (no change-doc or slice ids; ADR links only):
  - `docs/developer-guide.md`: in the relay-operator passport section, add
    "Use a passport on your substrate": the bootstrap order, the
    `home-relay set` and `status` commands, the `--ca-cert` case, the
    renewal and the warning, the `access_ends_at_secs` gauge (`0` means no
    active passport), the state file
    `<storage.db_dir>/home_relay_passport.json`, and that a node on a
    passport relay is reachable only by a direct path or by leased peers.
    Also: a passport relay serves its relay over plain HTTP or with a
    WebPKI-trusted certificate (Q13); and a node that uses a passport relay
    should not run a coordinator Iroh role with the same parent relay
    (Q14). **Remove** the sentence at ~line 1817 ("Substrate-side
    configuration ... is added in subsequent work").
  - `docs/system-architecture.md`: the substrate side of the passport (the
    agent task, register-then-insert order, the control-plane methods). Add
    `substrate.home_relay.*` (the `access_ends_at_secs` gauge) to the list
    of metric families (~line 1446).
  - `docs/system-requirements-spec.md`, Relay section: H-4, H-5, H-7 as
    requirements.
  - `docs/planning/traceability-matrix.md`: find the requirement id whose
    text holds the Relay passport paragraph of the requirements spec, and
    add the B3 evidence (code and the e2e test) to its row. Do not invent a
    new id; if no row carries the relay passport yet, add the B1 to B3
    evidence to that row.
  - `docs/TERMINOLOGY.md`: B2 already added Passport, Child token and Lease.
    Add "Info URL" (the base URL of a coordinator's info server, not its
    relay URL), because the guide and `roymctl` now use it as a term.
- `docs/planning/deferred-backlog.md`:
  - Update the three rows of §2.1 item 4 (move to "Recently resolved" with
    how B3 resolved its half, or reword what is left).
  - New rows: (1) the owner cannot reach a node on a passport relay through
    `roymctl` (Q2), next to "Visitors need a lease on a passport relay";
    (2) no `home-relay clear` verb (Q11); (3) a passport for a relay other
    than the configured one needs a config change and a restart (Q1/Q2);
    (4) a relay with `[roles.coordinator] tls` and a private or self-signed
    certificate cannot be a home relay: the substrate's Iroh endpoint
    trusts only WebPKI roots (Q13, `crates/router/src/net_iroh.rs`
    `build_iroh_endpoint`); (5) a node's own coordinator role takes
    `parent_coordinator.iroh.url` as its parent relay, so it is refused by a
    passport relay (Q14; extends the B2 note that only an open relay can be a
    parent relay); (6) any shortcut taken while coding.
  - Q15's reading of "reconnect after expiry" is a `D-B3-*` decision in the
    change doc, not a backlog row.
- `task.md`: header status line ("S0, A1 to A4, C1, B1 to B3 are done"), B3
  "Done <date>. Decisions `D-B3-*` are in the passport change doc."
  `status.md`: the B3 row with the evidence (test files and counts, gates).

## 5. Completion steps

1. Import clean-up pass over every edited file (types by `use`, functions
   qualified by their parent module, no inline multi-`::` paths).
2. `cargo +nightly-2026-04-06 fmt --all`.
3. `cargo clippy -q --workspace --all-targets --all-features`.
4. `cargo nextest run -p syneroym-passport -p syneroym-control-plane -p syneroym-substrate -p roymctl`
   while iterating (the integration test needs the sandbox off: it binds
   ports and spawns `openssl`).
5. Duplication: run the `cargo dupes` recipe from `AGENTS.md` over the
   changed files. Watch the moved `register_test_passport` and the HTTP
   answer mapping.
6. Ratchets: if a gate says a value got better, record the new value in the
   same change (`xtask/src/duplication.rs`, the lint-suppression count, the
   oversized-test list).
7. `mise run verify` in the background, no `--skip`, and read the summary.
8. Deferred-backlog pass (Step 10) and living-docs pass.
9. Do not commit on `main`. On the feature branch, commit only when the owner
   asks.
