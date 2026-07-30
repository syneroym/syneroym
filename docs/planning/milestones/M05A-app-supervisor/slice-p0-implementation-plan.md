# Slice P0 Implementation Plan — `ControllerAgreement` Creation, the `security` Gate, and the Fail-Closed Default

**Status:** 📋 Planned (2026-07-29). Not started. Milestone:
[task.md](task.md) slice **P0**. Design of record: none of its own — P0 is
pulled forward from
[M04A Slice B7](../M04A-proxy-and-auth-foundation/plans/B7.md) §6.1 item 1 /
§6.2, F3.1, and F4/Q1. [ADR-0015](../../../decisions/0015-ucan-capability-model.md)
§1-§2 and amendment A6 define the ability vocabulary and the trusted-root
rule this slice switches on. Gates slice A3.

**Not quite independent of A0-A2**, despite what P0's numbering suggests:
`resolve-instance-identity` is gated on `orchestrator/status`
([orchestration.rs:545-556](../../../../crates/control_plane/src/service/orchestration.rs#L545)),
and A0's operator flow `roymctl identity certify-instance` calls it
([member_identity.rs:113](../../../../apps/roymctl/src/commands/member_identity.rs#L113)).
After Phase 3 that command fails outright on an unowned substrate and needs
`--as <controller>` on a claimed one. P0 changes no A0-A2 *code*, but it does
change how an A0 flow is invoked — see §5.6.

**Read §0 first.** Planning found eleven places where `task.md` describes a
tree that does not exist, leaves a decision unmade, or understates the blast
radius. **Five** of them change what P0 has to build. §1's decisions
(D-P0-1 … D-P0-12) take §0's recommended resolutions as given.

**Review round 2 (2026-07-29), all findings incorporated.** Round 1's own fix
was undercounted: `tests/perf` has **five** orchestrator clients, not four —
the fifth (`soak.rs:276`) sits inside the deploy-churn loop's spawned task and
is the only `undeploy` caller in the crate, so `bench:soak` would still have
failed after the round-1 fix, on the very gate §6 added to catch it. §5.7 now
lists all five, and also lists the **six** app-targeting clients that must
*not* be swapped (a mechanical find-and-replace over
`SyneroymClient::new` breaks the suite differently). Three smaller fixes: the
"check the harness key-write ordering" hedge is answered by
`TestEnvironment::new`'s existing `pub substrate_identity`
([orchestrator.rs:59-72](../../../../tests/perf/src/orchestrator.rs#L59)), so
the pseudo-code no longer reloads the key from disk; §10's `issue-grant`
example was missing the required `--expires-days`; and §9's harness count
still said eight.

**Review round 1 (2026-07-29), all findings incorporated.** One whole harness
was missing (`tests/perf`, §0.3a — and it is invisible to every gate the plan
originally listed, so §6 now names `mise run bench:latency` explicitly); a
third deployer in `federated_fdae_e2e.rs` went unrooted (§5.3's
`bad_app_deployer`); §5.3's app-scoped grant was deploy-only and would have
tripped `deploy`'s own `undeploy` rollback path
([orchestration.rs:1255-1269](../../../../crates/control_plane/src/service/orchestration.rs#L1255)),
whose comment names this exact moment as the one to revisit; the
stale-comment sweep listed 4 sites and there are 16 (§3.3); and
`certify-instance` is an A0 flow this slice changes the invocation of (§5.6),
which the original header wrongly called "independent of A0-A2". Also
corrected: `mise run test:e2e` already covers the multi-hop config, and
`config.dev.toml` needs the same edit as `config.sample.toml`.

**The one-sentence summary.** Three changes ship together: `roymctl substrate
claim` mints a mutually-signed `ControllerAgreement` from two local key files;
the `security` interface starts requiring `substrate/admin`; and
`build_caller` stops handing `orchestrator/{deploy,undeploy,status}` to every
verified caller on an unowned substrate. Each alone is inert or a regression
(task.md's P0 §Scope); chained, they work.

---

## §0 — What `task.md` gets wrong, leaves open, or understates

Same discipline as A0 §6 / A1 §6 / A2 §0: recorded here rather than silently
worked around, so `task.md` (and, where relevant, B7's plan) can carry a dated
correction at sign-off.

### 0.1 (Scope-changing) A self-owned agreement can never verify — the tool must reject it

`SubstrateIdentityState::init`
([substrate.rs:213-223](../../../../crates/identity/src/substrate.rs#L213))
walks the proof list with an `if / else if`:

```rust
if proof.verification_method.starts_with(&agr.controller) { ... controller_valid = true }
else if proof.verification_method.starts_with(&substrate_did) && ... { substrate_valid = true }
```

When `controller == controlled` (an operator "claiming" a node with the node's
own key), **every** proof matches the first branch, `substrate_valid` stays
`false`, and the agreement is reported `Unverified` — i.e. the node stays
unowned, silently, after a command that printed success.

**Resolution:** the tool rejects `--controller` whose resolved DID equals the
node DID, with an explicit error. Do **not** "fix" the proof loop to accept
self-ownership: a node that owns itself means anyone holding the node key
holds `substrate/admin`, which is exactly the property the controller/
controlled split exists to separate.

### 0.2 (Scope-changing) Running the tool changes nothing unless the config is also edited

`setup_substrate_identity`
([identity.rs:38-47](../../../../crates/substrate/src/identity.rs#L38)) loads
an agreement **only** when `config.identity.agreement` is `Some(path)`. It is
set in no config file in the tree (`config.sample.toml` documents it, commented
out; `config.dev.toml` and both e2e configs omit it). So "run the P0 verb" is
a two-step operation with an undocumented second step, and the failure mode of
forgetting it is silent (the node boots unowned).

**Resolution:** `setup_substrate_identity` falls back to
`<app_data_dir>/agreement.json` when `config.identity.agreement` is `None` and
that file exists — the same implicit-discovery rule `identity.key` already
follows via `DEFAULT_SUBSTRATE_KEY_FILE`. `claim` writes there by default.
Explicit config still wins.

### 0.3 (Scope-changing) The blast radius of the fail-closed flip is nine harnesses, not "reconsider a default"

task.md item 3 reads as a one-line policy change. It is not. Everything below
today reaches `orchestrator/deploy` (and `security`) purely through the unowned
bootstrap posture:

| Harness | File | What it needs |
|---|---|---|
| Shared `SubstrateTestContext` | `crates/substrate/tests/common/mod.rs` (consumed by `http_passthrough_e2e.rs`, `stream_client_e2e.rs`, `messaging_client_e2e.rs`) | owner identity + `admin_ucan_root` |
| `basic_lifecycle`'s own copy | `crates/substrate/tests/basic_lifecycle.rs:160-250` | same |
| `podman_lifecycle`'s own copy | `crates/substrate/tests/podman_lifecycle.rs:26-130` | same |
| `Node` (two-node) | `crates/substrate/tests/federated_fdae_e2e.rs:74-180` | **see 0.4 — not mechanical** |
| `Node` (two-node) | `crates/substrate/tests/instance_identity_e2e.rs:74-140` | owner identity + `admin_ucan_root` |
| `Node` (two-node) | `crates/substrate/tests/master_endpoint_record_e2e.rs:87-150` | same |
| Playwright single-node | `crates/substrate/tests/e2e/global-setup.ts` | real `claim` flow + `--as` |
| Playwright multi-hop | `crates/substrate/tests/e2e/global-setup-multihop.ts` (nodes `sz`, `sx`) | same, ×2 |
| **Performance harness** | `tests/perf` (`syneroym-perf`) | **see 0.3a — caught by no gate in §6** |

Plus **12 `inject_kek` call sites** across five of those files, all of which go
through an ephemeral `SyneroymClient::new` identity and would be denied by the
new `security` gate.

### 0.3a (Scope-changing) The performance harness breaks, and no gate catches it

`tests/perf` starts its substrate as `syneroym-substrate run --key <tmpfile>`
with no `--config`
([orchestrator.rs:103-112](../../../../tests/perf/src/orchestrator.rs#L103)),
so it falls through to `dev_mode_config()`
([main.rs:190](../../../../crates/substrate/src/main.rs#L190)) — no
`admin_ucan_root`, no agreement. Five orchestrator clients across four scenarios then
build an ephemeral `SyneroymClient::new` and deploy:
[tcp_proxy_latency.rs:54](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L54),
[concurrency.rs:50](../../../../tests/perf/src/scenarios/concurrency.rs#L50),
[wasm_latency.rs:151](../../../../tests/perf/src/scenarios/wasm_latency.rs#L151),
[soak.rs:87](../../../../tests/perf/src/scenarios/soak.rs#L87), and
[soak.rs:276](../../../../tests/perf/src/scenarios/soak.rs#L276) inside the
deploy-churn loop. All five are denied at deploy after Phase 3, breaking `mise run bench:latency`,
`bench:concurrency`, `bench:soak`, `cargo xtask perf-summary`, and the
`benchmarks` job in `.github/workflows/ci.yml:96`.

**This is the sharpest failure mode in the slice**: it compiles clean,
`cargo test --workspace` never runs it, and `mise run test:e2e` does not touch
it — so every gate in §6 passes while the benchmark suite is dead. It must be
in the gate list explicitly (§6) and fixed in §5.7.

The fix is **not** a config edit (there is no config file). The route is to
mint an agreement in the harness and pass the `run --agreement <path>` flag
that already exists
([main.rs:56-58](../../../../crates/substrate/src/main.rs#L56)). Note that
implicit discovery (D-P0-5) does **not** help here: `--key` points at a temp
file while `app_data_dir` stays at the dev-mode default, so the agreement
would be looked for somewhere unrelated to the key.

### 0.4 (Scope-changing) The flip silently changes what `federated_fdae_e2e` proves

`federated_fdae_e2e.rs` deliberately boots **Node B unowned**
([:455-464](../../../../crates/substrate/tests/federated_fdae_e2e.rs#L455)),
and its assertions depend on "alice holds no capability at all on Node B
without self-issuing one". The tempting fix — set `admin_ucan_root =
alice_did` — gives alice `substrate/admin`, which entails `data-layer/write`
on every resource there (`Ability::entails`'s short-circuit,
[capability.rs:138-141](../../../../crates/ucan/src/capability.rs#L138)),
making the file's carefully separated read-only/write-only token pair
meaningless while every assertion still passes.

**Resolution:** Node B gets a **distinct** owner identity, and `alice_deployer`
presents an app-scoped `orchestrator/deploy` grant issued by that owner
(`substrate:<node_b_did>/app/<app_service_id>`) — the exact shape B7b built and
`deploy_grant.rs` already exercises. Detailed in §5.3.

### 0.5 `roymctl substrate init` writes the wrong filename

`SubstrateCommands::Init` writes `<dir>/identity.key`
([substrate.rs:29](../../../../apps/roymctl/src/commands/substrate.rs#L29)),
while `DEFAULT_SUBSTRATE_KEY_FILE` is `"substrate.key"`
([config.rs:9](../../../../crates/core/src/config.rs#L9)) and
`get_substrate_did` looks for `<dir>/substrate.key`
([commands.rs:93](../../../../apps/roymctl/src/commands.rs#L93)). Both e2e
setups paper over it with an explicit `key = "identity.key"`. The claim tool
must locate the node key, so this trap becomes load-bearing.

**Resolution:** change `Init` to write `substrate.key` and update both e2e
configs. Pre-release, in place, no shim.

### 0.6 The flip *resolves* an open backlog row, and nothing says so

[deferred-backlog.md](../../deferred-backlog.md) §3 carries
*"`resolve_relation`'s A1/A2 fork is defeated on an unowned substrate"* — the
free bare `substrate:<node_did>` capabilities make `resolve_relation`'s B3-07
fork always take A1. With the unowned grant gone, the row's cause is gone.
Move it to *Recently resolved* as part of this slice, and drop the workaround
comment it forced on `federated_fdae_e2e.rs:140-150`.

### 0.7 Line references in `task.md` have drifted

- P0 cites `io.rs:185-200` for the unowned grant. Actual: the comment starts
  at [io.rs:185](../../../../crates/router/src/route_handler/io.rs#L185) and
  the `vec![...]` at 198-202. Close enough to keep.
- P0 cites `service.rs:256-260` for the `TODO(M04B/FDAE)`. Actual:
  [service.rs:265-269](../../../../crates/control_plane/src/service.rs#L265)
  (line 256 is the closing brace of `has_node_wide_ability`). B7's F3.1 cites
  `service.rs:118-122` for the same TODO — also stale.

Update both citations when P0 lands, or drop them for symbol names.

### 0.8 Nothing says *where* the tool runs — and the answer constrains A3

Producing an agreement needs **both** private keys: the controller's and the
node's. The node's private key exists only on the node's filesystem, and no RPC
returns a signature over caller-supplied bytes. So the tool is inherently a
**local, offline** operation performed on the substrate host — there is no
remote claim, and there cannot be one without a new (TOFU, first-claim-wins)
substrate endpoint.

**Consequence for A3:** provisioning N substrates means visiting N hosts (or
shipping `agreement.json` out of band). That is acceptable for P0 and worth a
backlog row, not a redesign. task.md is silent on it; it should not be.

### 0.9 `agreement.type` is never validated, and a malformed `expiresAt` fails open

- `ControllerAgreement.agreement_type` (JSON `"type"`) is read by nothing. The
  tool has to pick a value; recommend `"ControllerAgreement"` and enforce it in
  `SubstrateIdentityState::init` (one-line tightening, pre-release).
- The expiry check is `if let Ok(dt) = DateTime::parse_from_rfc3339(expires_at)`
  ([substrate.rs:227](../../../../crates/identity/src/substrate.rs#L227)) —
  an unparseable `expiresAt` is treated as *no expiry*, i.e. fail-open on a
  hand-edited agreement. Recommend making a present-but-unparseable
  `expiresAt` an error (or `Unverified` when `require_agreement` is false).

Both are small and belong here, since P0 is the first code that produces the
artifact these checks read.

Two adjacent warts found in the same files, **flagged not fixed** unless the
requester says otherwise:

- Both `config.sample.toml` and `config.dev.toml` suggest the filename
  `substrate-controller-agreement.json` in their commented-out `agreement =`
  line. D-P0-5 picks the shorter `agreement.json` for the discovery default;
  nothing depends on the sample's name (the line is commented out), but the
  comment text must be updated so the two do not disagree.
- Both sample configs set `controller_did = ""`. That reaches
  `SubstrateIdentityState::init` as `Some("")`, producing
  `controller: Some(""), status: Unverified`. Harmless today (only a
  `Verified` controller becomes `admin_ucan_root`), but it is a non-DID
  masquerading as a controller in the boot log. Either drop the line from the
  samples or treat an empty `controller_did` as `None`.

### 0.10 The `security` gate needs a caller-shape decision the docs never make

`security` is reached only over the wire today (12 test call sites + `roymctl
kek`/`secret`; **no** substrate-internal dispatch — verified by grep for
`inject_kek`/`rotate_kek`/`set_secret`/`"set-secret"`). So the gate needs no
`AuthLevel::System`/`LocalElevated` exemption, and adding one pre-emptively
would be a hole. Recorded because "gate on `substrate/admin`" alone does not
say whether substrate-injected callers are exempt — they are not.

---

## §1 — Decisions

| # | Decision |
|---|---|
| **D-P0-1** | The verb is **`roymctl substrate claim`** (aliased `roymctl node claim`, via the existing `#[command(alias = "node")]`). It lives in `commands/substrate.rs`, whose handler already takes only `&dir` — the whole operation is local file I/O plus two signatures. Rejected: `identity create-agreement` (the artifact is about a *node*, not an identity). |
| **D-P0-2** | The tool signs **both** proofs in one invocation, from two local key files. No remote claim endpoint, no partial/detached-signature flow. (§0.8) |
| **D-P0-3** | `--controller` names a **local identity** under `<dir>/identities/<name>.key` (same lookup as `--as` / `identity delegate --master`), never a bare DID — a bare DID could not sign. |
| **D-P0-4** | Reject `--controller` resolving to the node's own DID (§0.1), with an error naming the reason. |
| **D-P0-5** | Output defaults to `<dir>/agreement.json`; `setup_substrate_identity` discovers `<app_data_dir>/agreement.json` implicitly when `[identity].agreement` is unset (§0.2). A new `DEFAULT_CONTROLLER_AGREEMENT_FILE` const sits beside `DEFAULT_SUBSTRATE_KEY_FILE`. An existing output file is **not** overwritten without `--force`. |
| **D-P0-6** | An unparseable implicitly-discovered `agreement.json` is a **hard boot failure**, same as an explicitly-configured one. Silently continuing unowned is exactly the failure mode P0 exists to remove. |
| **D-P0-7** | `agreement_type` is the literal `"ControllerAgreement"`, enforced at verification (§0.9). |
| **D-P0-8** | The `security` gate is `substrate/admin` on the **bare** `substrate:<node_did>` resource, checked with the existing `ControlPlaneService::has_node_wide_ability`. No app-scoped variant: injecting a KEK unlocks every service DB on the node, so there is no meaningful narrower resource. No exemption for substrate-injected callers (§0.10). **Consequence, stated here so it is not discovered at A3/A5:** all three methods — including `set-secret`, which *is* per-service — become node-owner-only, so a B7b app-scoped deploy grantee can deploy a service but cannot set a secret for it. `task.md` does not require otherwise and B7 F3.1 argues for exactly this coupling (a KEK is not less privileged than node admin), but the asymmetry is real. A per-service `set-secret` gate is a possible later split; it is not P0's. |
| **D-P0-9** | Denial is `RpcError::Custom(-32010, …)` — the code `synsvc_native.rs` already uses for permission denied — not `InternalError`. A distinct code is what lets a test assert *denial* rather than string-match. The literal is promoted to `syneroym_rpc::PERMISSION_DENIED_CODE` and both sites use it. |
| **D-P0-10** | Fail-closed is **unconditional**: no `[iam].allow_unowned_deploy` escape hatch. A flag would leave the flip untested in every harness that sets it, and matrix rows 16/17 would have no live proof. The cost is the §5 sweep, taken deliberately. |
| **D-P0-11** | Test harnesses adopt ownership by giving each node an **owner `Identity`** and building `substrate_client` from it (`SyneroymClient::new_with_identity`), so the 12 `inject_kek` and ~10 deploy call sites need **no** edits. Only harness constructors change. |
| **D-P0-12** | The Playwright e2e setups use the **real `claim` flow** (not `admin_ucan_root` in the TOML), because they are the only place the end-to-end operator path gets exercised. |

---

## §2 — Phase 1: the tool

Independently mergeable. Adds a capability; changes no posture. Merge first.

### 2.1 `crates/core/src/config.rs`

```rust
pub const DEFAULT_SUBSTRATE_KEY_FILE: &str = "substrate.key";
/// Implicitly discovered under `app_data_dir` when `[identity].agreement`
/// is unset -- `roymctl substrate claim`'s default output path, so claiming
/// a node and restarting it establishes ownership with no config edit.
pub const DEFAULT_CONTROLLER_AGREEMENT_FILE: &str = "agreement.json";
```

No struct change: `IdentityConfig.agreement` stays `Option<PathBuf>`, and
`resolve_paths` already joins a relative value onto `app_data_dir`
([config.rs:105-109](../../../../crates/core/src/config.rs#L105)).

### 2.2 `crates/identity/src/substrate.rs` — an `issue` constructor

New, next to `from_json`:

```rust
/// The only `agreement_type` this tree issues or accepts.
pub const CONTROLLER_AGREEMENT_TYPE: &str = "ControllerAgreement";

impl ControllerAgreement {
    /// Mint a mutually-signed agreement binding `node`'s DID (the
    /// `controlled`) to `controller`'s DID. Both proofs are produced here
    /// because both private keys are needed and neither can be supplied
    /// remotely: the node's key lives only on the node's filesystem.
    ///
    /// `expires_in_secs: None` issues an agreement with no expiry.
    pub fn issue(
        node: &Identity,
        controller: &Identity,
        expires_in_secs: Option<u64>,
    ) -> Result<Self> { ... }
}
```

Pseudo-code:

```
node_did       = derive_did_key(node.public_key())
controller_did = derive_did_key(controller.public_key())

if node_did == controller_did:
    bail!("a substrate cannot be its own controller: the agreement's two \
           proofs are indistinguishable when `controlled` == `controller`, \
           so it can never verify (see SubstrateIdentityState::init). \
           Create a separate operator identity with `roymctl identity \
           create --name <name>`.")                                  # D-P0-4

now       = Utc::now()
issued_at = now.to_rfc3339_opts(SecondsFormat::Secs, true)
expires_at = expires_in_secs.map(|s| (now + Duration::seconds(s as i64))
                                      .to_rfc3339_opts(SecondsFormat::Secs, true))

# Build the unsigned agreement, sign the *proof-less* canonical form --
# byte-for-byte what `verify_signature` reconstructs
# (substrate.rs:150-157): serde_json::to_value(agreement),
# remove "proof", canonicalize_json_value, serialize.
unsigned = Self { agreement_type: CONTROLLER_AGREEMENT_TYPE.into(),
                  controlled: node_did, controller: controller_did,
                  issued_at, expires_at, proof: vec![] }

payload = { let mut v = serde_json::to_value(&unsigned)?;
            v.as_object_mut().unwrap().remove("proof");
            canonicalize_json_value(&v) }

proofs = vec![
    proof_for(controller, &controller_did, &payload)?,   # controller side
    proof_for(node,       &node_did,       &payload)?,   # substrate side
]
Ok(Self { proof: proofs, ..unsigned })
```

with

```rust
fn proof_for(signer: &Identity, signer_did: &str, payload: &Value) -> Result<Proof> {
    Ok(Proof {
        proof_type: "Ed25519Signature2020".to_string(),   // the only type verify_signature accepts
        verification_method: format!("{signer_did}#key-1"), // `starts_with(did)` is what init checks
        proof_purpose: "capabilityDelegation".to_string(),  // unread today; conventional
        proof_value: signer.sign_json(payload)?,            // z-base-32, RFC-8785 canonical
    })
}
```

> **Invariant to preserve.** `Identity::sign_json` canonicalizes internally
> ([keys.rs:201-206](../../../../crates/identity/src/keys.rs#L201)), and
> `canonicalize_json_value` is idempotent, so passing an
> already-canonicalized `payload` is safe. Do **not** hand `sign_json` the
> agreement *with* `proof` present — `verify_signature` removes it before
> checking.

`ControllerAgreement` must gain `#[derive(PartialEq)]`? No — not needed;
tests compare fields.

### 2.3 `crates/identity/src/substrate.rs` — two verification tightenings

Both inside `SubstrateIdentityState::init`, at the top of the
`if let Some(agr) = agreement` block:

1. **Type check (D-P0-7)**, immediately before the existing
   `agr.controlled != substrate_did` check:

```
if agr.agreement_type != CONTROLLER_AGREEMENT_TYPE:
    require_agreement ? Err("unsupported agreement type '{}'")
                      : Ok(state{controller: None, status: None})
```

2. **Expiry parse (§0.9)**, replacing the `if let Ok(dt)` at
   [substrate.rs:226-238](../../../../crates/identity/src/substrate.rs#L226):

```
if let Some(expires_at) = &agr.expires_at:
    dt = DateTime::parse_from_rfc3339(expires_at)
         or -> require_agreement ? Err("agreement expiresAt is not RFC-3339")
                                 : Ok(state{controller: Some(..), status: Unverified})
    if dt < Utc::now(): (existing expired handling, unchanged)
```

### 2.4 `crates/substrate/src/identity.rs` — implicit discovery (D-P0-5/D-P0-6)

Replace lines 37-47:

```rust
// D-P0-5: an explicit `[identity].agreement` wins; otherwise the
// substrate picks up `<app_data_dir>/agreement.json` if it exists --
// `roymctl substrate claim`'s default output, so claim-then-restart
// establishes ownership with no config edit.
let agreement_path = config
    .agreement
    .clone()
    .unwrap_or_else(|| app_data_dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE));

let agreement = if agreement_path.exists() {
    let json = fs::read_to_string(&agreement_path)
        .with_context(|| format!("failed to read controller agreement at {}",
                                 agreement_path.display()))?;
    // D-P0-6: a present-but-unparseable agreement is a hard failure on
    // both the explicit and the discovered path. Booting unowned because
    // the ownership artifact was malformed is the exact silent failure
    // this slice removes.
    Some(ControllerAgreement::from_json(&json)
        .with_context(|| format!("invalid controller agreement at {}",
                                 agreement_path.display()))?)
} else {
    None
};
```

Note the behavior change on the *explicit* path too: today a configured-but-
missing agreement path is silently `None`. It stays silently `None` (the path
does not exist), which is fine; what changes is that a *malformed* file is now
an error rather than… actually it already was (`?` on `from_json`). Only the
`with_context` wrapping and the discovery fallback are new.

Requires `use anyhow::Context;` and
`syneroym_core::config::DEFAULT_CONTROLLER_AGREEMENT_FILE`.

### 2.5 `apps/roymctl/src/commands/substrate.rs` — the verb

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum SubstrateCommands {
    Init,
    /// Establish ownership of this substrate: mint a mutually-signed
    /// `ControllerAgreement` binding the node's DID to a controller
    /// identity you hold. Must run on the substrate host -- it reads the
    /// node's own private key, which never leaves that filesystem.
    ///
    /// The substrate picks the agreement up on its next start (from
    /// `<dir>/agreement.json`, or from `[identity].agreement` if set), and
    /// from then on only the controller holds `substrate/admin`: deploy,
    /// undeploy, status, and the `security` interface (KEK/secrets).
    Claim {
        /// Local identity that becomes the controller (see `roymctl
        /// identity create --name`). Must not be the node's own key.
        #[arg(long)]
        controller: String,
        /// The node's private key. Defaults to `<dir>/substrate.key`.
        #[arg(long)]
        substrate_key: Option<PathBuf>,
        /// Where to write the agreement. Defaults to `<dir>/agreement.json`,
        /// which the substrate discovers with no config change.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Expiry in days. Omitted means no expiry.
        #[arg(long)]
        expires_days: Option<u64>,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },
    Status,
    Config,
}
```

Handler arm:

```
SubstrateCommands::Claim { controller, substrate_key, out, expires_days, force } => {
    node_key_path = substrate_key.clone()
                    .unwrap_or_else(|| dir.join(DEFAULT_SUBSTRATE_KEY_FILE))
    if !node_key_path.exists():
        bail!("no substrate key at {}. Run `roymctl substrate init --dir {}` \
               first, or pass --substrate-key <path>. `claim` must run on the \
               substrate host: it signs with the node's own key.")

    controller_path = dir.join("identities").join(format!("{controller}.key"))
    if !controller_path.exists():
        bail!("no local identity '{controller}' at {}. Create one with \
               `roymctl identity create --name {controller}`.")

    node       = Identity::load_from_path(&node_key_path)?
    controller_id = Identity::load_from_path(&controller_path)?

    out_path = out.clone().unwrap_or_else(|| dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE))
    if out_path.exists() && !force:
        bail!("{} already exists; pass --force to replace it. Replacing an \
               agreement transfers ownership on the next substrate start.")

    agreement = ControllerAgreement::issue(&node, &controller_id,
                                           expires_days.map(|d| d * 24 * 3600))?
    fs::write(&out_path, serde_json::to_string_pretty(&agreement)?)?

    println!("Substrate claimed.")
    println!("  node DID:       {}", agreement.controlled)
    println!("  controller DID: {}", agreement.controller)
    println!("  agreement:      {}", out_path.display())
    println!()
    println!("Restart the substrate to apply -- it reads the agreement once, at \
              boot. It is picked up from {out_path} automatically; a substrate \
              started with a different data directory can be pointed at it with \
              `syneroym-substrate run --agreement {out_path}`. From then on, \
              control it with `roymctl --as {controller} ...`.")
}
```

The `run --agreement <path>` flag already exists
([main.rs:56-58](../../../../crates/substrate/src/main.rs#L56)) and overrides
`[identity].agreement`; it is the alternative to editing a config file, not an
alternative to restarting. A restart is required either way (§8's backlog row).

`SubstrateCommands::Init` also changes: `dir.join("identity.key")` →
`dir.join(DEFAULT_SUBSTRATE_KEY_FILE)` (§0.5), and the success message names
the file.

New imports in that module: `std::path::PathBuf`,
`syneroym_core::config::{DEFAULT_CONTROLLER_AGREEMENT_FILE, DEFAULT_SUBSTRATE_KEY_FILE}`,
`syneroym_identity::substrate::ControllerAgreement`, `anyhow::bail`.
`apps/roymctl/Cargo.toml` already depends on `syneroym-core` and
`syneroym-identity` — verify before adding.

### 2.6 Docs touched by Phase 1

- `crates/substrate/config.sample.toml:32-36` **and
  `crates/substrate/config.dev.toml:32-36`** — identical blocks; both must
  change. The `agreement` comment now says the file is discovered by default at
  `<app_data_dir>/agreement.json` (and stops suggesting the name
  `substrate-controller-agreement.json`, §0.9). Both also carry
  `controller_did = ""` — drop the line or leave it with a comment, per §0.9.
- `docs/developer-guide.md` — three edits, not one:
  1. A new "Claiming a substrate" run-through (init → `identity create` →
     `substrate claim` → start → `--as`), best placed in §4 just after
     *Identifying your Substrate* (~:171) and before *Managing Identities*,
     since everything below it now depends on ownership.
  2. §4's orchestrator walkthroughs (*List Deployed Services* ~:219 through
     *Deploy a Container Service* ~:349, plus *Developing Podman Services
     Locally* ~:433) — every `roymctl` invocation there hits `orchestrator`
     and needs `--as <controller>`. Without this the guide's examples all
     fail after Phase 3.
  3. §5.1 *Cryptographic Identity Delegation* (~:485-515) — the
     `identity delegate` flow, and the `certify-instance` note from §5.6.

---

## §3 — Phase 2: gate `security` on `substrate/admin`

Depends on Phase 1 being *available* (the gate is holdable), not merged.
Mergeable together with Phase 1 in one PR; not before it.

### 3.1 `crates/rpc/src/lib.rs`

```rust
/// JSON-RPC application error code for an authorization denial. Shared so a
/// caller can distinguish "denied" from "failed" without string-matching.
pub const PERMISSION_DENIED_CODE: i32 = -32010;
```

Update `crates/control_plane/src/synsvc_native.rs:184` to use it in place of
the literal `-32010` (and the two `matches!` assertions at :1429/:1433 that
pattern-match the literal — a `const` cannot appear in a pattern, so those
stay literals with a comment, or become `if` comparisons).

### 3.2 `crates/control_plane/src/service.rs` — the gate

Replace the `TODO(M04B/FDAE)` block at
[service.rs:264-270](../../../../crates/control_plane/src/service.rs#L264):

```rust
if invocation.interface.as_str() == SECURITY_INTERFACE {
    // KEK injection/rotation and vault writes are node-owner operations:
    // a KEK unlocks every service database on this node, so there is no
    // meaningful resource narrower than the node itself to scope this to.
    // The gate is `substrate/admin` on the bare `substrate:<node_did>`
    // resource -- holdable only by a verified `ControllerAgreement`
    // controller (or `[iam].admin_ucan_root`). No exemption for
    // substrate-injected callers: nothing inside the substrate dispatches
    // to this interface.
    if !self.has_node_wide_ability(&invocation.caller, Ability::SUBSTRATE_ADMIN) {
        return Err(RpcError::Custom(
            PERMISSION_DENIED_CODE,
            format!(
                "caller {} holds no substrate/admin on this substrate; the \
                 security interface is node-owner only",
                invocation.caller.caller_did
            ),
            None,
        ));
    }
    match invocation.method.as_str() { /* unchanged */ }
}
```

`has_node_wide_ability`'s doc comment
([service.rs:221-252](../../../../crates/control_plane/src/service.rs#L221))
must be amended: it currently reads as orchestrator-only ("Whether `caller`
holds a specific **node-wide** orchestrator ability"). Generalize the first
line and add a sentence that `security` passes `SUBSTRATE_ADMIN` here. Its
"on an unowned substrate, any verified caller" clause becomes **wrong** at
Phase 3 and must go in the same pass.

Imports: `Ability` is already in scope in `service.rs` (used at :479);
`PERMISSION_DENIED_CODE` is new from `syneroym_rpc`.

### 3.3 Stale comments retargeted in the same pass

Every comment below asserts something Phase 2 or Phase 3 makes false. This
list is exhaustive as of 2026-07-29; re-run
`rg -n 'unowned|F4|still-ungated' --type rust` before finishing to catch
anything added since.

| File:line | What it says that stops being true |
|---|---|
| `crates/router/src/route_handler/io.rs:147-156` | `TODO(B7b / post-B7)`: "`security`'s gate … ships with that tool". Drop that sentence; the remaining gap is the five data interfaces only. |
| `crates/router/src/route_handler/dispatch.rs:86-100` | Same TODO text, same edit. |
| `crates/control_plane/src/service.rs:221-252` | `has_node_wide_ability`'s doc: "on an unowned substrate, any verified caller". Also generalize it off "orchestrator" (§3.2). |
| `crates/control_plane/src/service.rs:456-463` | `node_wide_caller` test helper doc: "the shape `build_caller` issues for the F4 unowned-substrate bootstrap grant". |
| `crates/control_plane/src/service/orchestration.rs:658-666` | Takeover check "never fires" on an unowned substrate. |
| same, `:790-803` | Same claim for the app-instance owner check. |
| same, `:1255-1269` | The deploy-rollback/`orchestrator/undeploy` interaction is "inert today … Revisit if a deploy-only grantee becomes real before the ownership tooling lands." **P0 is that moment** — see §5.3, which must either widen the grant or state that the rollback path is denied. Not a comment-only edit. |
| same, `:1266` | "nothing can create a `ControllerAgreement` yet, so every substrate is unowned". |
| same, `:1446-1454` | `list`: "the substrate owner, or on an unowned substrate, everyone (F4)". |
| same, `:1497-1505` | `node_wide_caller` doc, same F4 framing. |
| `crates/ucan/src/capability.rs:38-42` | "at B7b this is exactly the shape `build_caller` still issues for the F4 unowned posture". |
| `crates/router/src/proxy.rs:228-233` | "including the unowned-substrate bootstrap grant … and `security`'s still-ungated dispatch (P0 item 2)". Both halves resolved here. |
| same, `:783-789` | Same wording in `check_native_capability_gate`'s doc. |
| `crates/router/tests/service_ownership.rs:7, 104, 349, 486, 544` | Doc comments framing the caller shape as "the F4 unowned-substrate grant". |
| same, `:354` | The **test name** `unowned_substrate_lists_every_app_to_any_caller`. The assertion still passes (hand-built caller), but the name states something Phase 3 makes untrue. Rename to `node_wide_authority_lists_every_app`. |
| `crates/client_gateway/src/gateway.rs:34-38` | `TODO(post-B0)`: "none exists yet — only ControllerAgreement" is now half-false. The gateway still presents the **node** DID, which under Phase 3 holds nothing node-wide. Keep the TODO; correct the parenthetical and note the new consequence. |

> **Flagged, not fixed:** whether the client gateway *should* present the
> controller DID is a real question P0 does not answer. Today it proxies only
> to deployed services, never to `orchestrator`/`security`, so nothing breaks
> — but that is an accident of routing, not a guarantee. Backlog row (§8).

### 3.4 Test updated by Phase 2

`crates/control_plane/src/service.rs`'s `test_security_dispatch_returns_sdk_statuses`
(~:665-700) drives all three methods with
`CallerContext::service_system("test-caller")`, which now denies. Add beside
the existing `node_wide_caller` helper (:468):

```rust
/// A caller holding `substrate/admin` on the test node -- what a verified
/// `ControllerAgreement` controller gets from `build_caller`. Separate from
/// `node_wide_caller` on purpose: `substrate/admin` entails *everything*,
/// so using it as generic deploy-test setup would make the orchestrator
/// gate tests prove less than they claim.
fn substrate_admin_caller(caller_did: &str) -> CallerContext { /* ResourceUri::substrate("did:key:zTestNode"), Ability::SUBSTRATE_ADMIN */ }
```

---

## §4 — Phase 3: fail-closed

Depends on Phases 1 and 2 being in the same PR (task.md's P0 §Scope: shipping
this alone bricks the substrate).

### 4.1 `crates/router/src/route_handler/io.rs` — `build_caller`

Replace lines 174-215 (the `node_wide_abilities` match and its `for` loop):

```rust
    let mut auth = AuthLevel::Delegated;

    // The substrate-owner capability is issued from this single site (M04A
    // Slice B7a's design §6.1.1: no "is this substrate owned?" branch
    // anywhere downstream). M05A Slice P0 removed the unowned bootstrap
    // grant that used to sit here: an unowned substrate issued
    // `orchestrator/{deploy,undeploy,status}` to every verified caller,
    // which was defensible while one operator hand-deployed to their own
    // node and is not once substrates are unattended networked deploy
    // targets. Bootstrap now happens off the wire entirely -- `roymctl
    // substrate claim` mints a `ControllerAgreement` from the node's own
    // key file on the node's own host -- so an unowned substrate can fail
    // closed without becoming unrecoverable.
    if admin_root == Some(id.master_did.as_str()) {
        session.capabilities.push(Capability {
            // Bare `substrate:<node_did>` -- the node itself, node-wide.
            with: ResourceUri::substrate(node_did),
            can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
            caveats: None,
        });
    }
```

Nothing else in the function changes. The UCAN branch below is unaffected and
becomes the *only* way a non-owner reaches `orchestrator/*` — which is what
B7b built it for.

The function's doc comment (:124-156) loses its unowned-posture paragraph.

### 4.2 `crates/substrate/src/runtime.rs` — the boot warning

Replace the `warn!` at
[runtime.rs:371-385](../../../../crates/substrate/src/runtime.rs#L371):

```rust
    if config.iam.admin_ucan_root.is_none() {
        warn!(
            "substrate has no verified ControllerAgreement controller and no \
             [iam].admin_ucan_root: running UNOWNED and FAIL-CLOSED -- no caller \
             can deploy, undeploy, status-check, or reach the security interface \
             (KEK/secrets) on this node. Establish ownership on this host with: \
             roymctl substrate claim --controller <name>  (then restart)"
        );
    }
```

The surrounding comment at :366-370 is rewritten to describe fail-closed.

### 4.3 Unit tests replaced in `io.rs`'s test module

| Test (:1086, :1116) | Fate |
|---|---|
| `unowned_substrate_grants_orchestrator_abilities_to_any_verified_caller` | **Replaced** by `an_unowned_substrate_grants_no_node_wide_capability` — asserts all three `orchestrator/*` **and** `substrate/admin` are absent on `ResourceUri::substrate(node_did)`. This is failure-matrix row 17's unit-level proof. |
| `unowned_substrate_does_not_grant_data_layer_admin` | **Kept**, comment updated (it now passes trivially, and that is worth stating rather than deleting the regression guard). |
| the owned-substrate test at :1140 | Unchanged. |

---

## §5 — Call-site sweep

Everything below exists only because Phases 2 and 3 change the posture. None
of it is optional; a missed harness is a red test suite.

### 5.1 The three single-node harnesses (mechanical, D-P0-11)

`crates/substrate/tests/common/mod.rs`, `basic_lifecycle.rs:160-250`,
`podman_lifecycle.rs:26-130` — each gets the same four edits:

```rust
// 1. mint an owner before the config is finalized
let owner = Identity::generate().expect("owner identity");
let owner_did = substrate::derive_did_key(&owner.public_key());

// 2. own the node
config.iam.admin_ucan_root = Some(owner_did.clone());

// 3. the harness client acts as the owner, so every existing
//    ctx.substrate_client.{deploy_*, inject_kek, request} call is
//    unchanged (D-P0-11)
let mut substrate_client = SyneroymClient::new_with_identity(
    substrate_service_id.clone(),
    registry_url.clone(),
    owner,
);

// 4. expose it for tests that build extra clients
pub owner_did: String,   // on SubstrateTestContext
```

New imports: `syneroym_identity::{Identity, substrate}` (`common/mod.rs`
currently imports neither).

Affected call sites needing **no** edit thanks to step 3: `http_passthrough_e2e.rs`
(:268, :330, :391, :443, :641 + every `deploy(&ctx.substrate_client, …)`),
`stream_client_e2e.rs` (:87, :89, :189), `messaging_client_e2e.rs` (:35),
`basic_lifecycle.rs` (:290), `podman_lifecycle.rs` (:205 and its deploy).

### 5.2 `instance_identity_e2e.rs` and `master_endpoint_record_e2e.rs`

Both already deploy as a real `operator: Identity` through
`orchestrator_client(node, caller)`. Both boot their nodes with no admin root.

Edit `Node::boot` in each to accept an `owner_did: String` and set
`config.iam.admin_ucan_root = Some(owner_did)`; pass the shared `operator`'s
DID from the test body. The node's own `substrate_client` (used for
`inject_kek` at `instance_identity_e2e.rs:219-220`,
`master_endpoint_record_e2e.rs:264-265`) must also become
`new_with_identity(..., operator)` — otherwise Phase 2 denies it.

> The `operator` identity is minted in the test body *after* `Node::boot`
> today. It must move **before** the boot calls in both files.

### 5.3 `federated_fdae_e2e.rs` — the non-mechanical one (§0.4)

- **Node A**: already owned by `hr_owner_did`. Change `Node::boot`'s
  `admin_ucan_root: Option<String>` parameter to `owner: Option<Identity>`,
  derive the DID inside, and build the node's `substrate_client` from it.
  Node A's `inject_kek` (:319) then works. `hr_deployer` (:328) already uses
  `hr_owner_identity` — unchanged.
- **Node B**: must stay *not owned by alice* to preserve what the file proves.
  Mint a `node_b_owner: Identity`, boot Node B owned by it, and:
  - `node_b.substrate_client.inject_kek` (:320) works via the owner client;
  - `alice_deployer` gets an app-scoped grant so it can still deploy:

```
// A helper, because Node B now needs this three times (§5.3's three deployers).
fn app_deploy_grant(node_owner: &Identity, grantee_did: &str,
                    node_did: &str, service_id: &str) -> CapabilityToken {
    let resource = ResourceUri(format!("substrate:{node_did}/app/{service_id}"));
    CapabilityToken::issue(
        node_owner,
        grantee_did,                             // audience == connecting identity
        // All three orchestrator abilities, matching the bundle F4's unowned
        // posture used to issue. `deploy` calls `self.undeploy(.., caller)`
        // on two rollback paths (orchestration.rs:1255-1269), so a
        // deploy-only grant would make a *failed* deploy fail again, on a
        // confusing second error. The abilities are flat and independently
        // grantable, but nothing here wants to prove that -- `deploy_grant.rs`
        // owns the partial-grant cases.
        [Ability::ORCHESTRATOR_DEPLOY, Ability::ORCHESTRATOR_UNDEPLOY, Ability::ORCHESTRATOR_STATUS]
            .into_iter()
            .map(|a| Capability { with: resource.clone(), can: Ability(a.to_string()), caveats: None })
            .collect(),
        Map::new(), 3600, vec![],
    ).expect("issue app deploy grant")
}

alice_deployer = SyneroymClient::new_with_identity(node_b.did(), node_b.registry_url,
                                                   Identity::from_bytes(&alice_identity.to_bytes()))
                    .with_ucan(app_deploy_grant(&node_b_owner, &alice_did,
                                                node_b.did(), &app_service_id));
```

  This is rooted correctly: `build_caller`'s `is_root` accepts
  `admin_root == issuer` when `resource_is_local` holds, and
  `substrate:<node_b_did>/app/<svc>` names Node B
  ([io.rs:224-229](../../../../crates/router/src/route_handler/io.rs#L224)).
  Alice gains `orchestrator/*` on exactly one app and **no** `data-layer/*`
  anywhere — the property §0.4 says must survive.
- **`bad_app_deployer`** ([:633-640](../../../../crates/substrate/tests/federated_fdae_e2e.rs#L633))
  is a *third* deployer on Node B, connecting as `alice_identity_2` to deploy
  `bad_app_service_id`. It needs its own grant, issued **to `alice_did_2`**:

```
bad_app_deployer = SyneroymClient::new_with_identity(node_b.did(), node_b.registry_url,
                                                     Identity::from_bytes(&alice_identity_2.to_bytes()))
                      .with_ucan(app_deploy_grant(&node_b_owner, &alice_did_2,
                                                  node_b.did(), &bad_app_service_id));
```

  **Do not** shortcut this by deploying `bad_app_service_id` as the node owner:
  `bad_seed_token` and `bad_query_client` further down self-issue owner-rooted
  tokens that only verify because `registry.owner_of(bad_app_service_id) ==
  alice_did_2` (ADR-0015 A6, and `deploy` records `caller.caller_did` as the
  owner). Changing who deploys silently breaks that root.
- Delete the workaround comment at :140-150 and :288-295 explaining why Node A
  is owned; with the unowned grant gone, `resolve_relation`'s A1/A2 fork is no
  longer defeated (§0.6). **Re-read the surrounding assertions before
  deleting** — if any of them only pass because Node A is owned for the *old*
  reason, the comment's removal is wrong and the finding belongs back in §0.

### 5.4 Playwright: `global-setup.ts` (D-P0-12)

After `node init` and before the substrate starts:

```ts
execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name owner`);
execSync(`"${ROYMCTL_BIN}" substrate claim --dir ${TEST_DIR} --controller owner`);
```

Config change: `[identity] key = "identity.key"` → `"substrate.key"` (§0.5).
No `agreement =` line is needed — discovery finds `<TEST_DIR>/agreement.json`
(D-P0-5). The substrate is started *after* the claim, so no restart is needed.

Then `--as owner` on the one substrate-authenticated command:

```ts
execSync(`... --substrate ${substrateDid} --as owner svc deploy --svc-id ${appDid} ...`);
```

`registry register` targets the community registry over HTTP and is **not**
affected.

### 5.5 Playwright: `global-setup-multihop.ts`

Four nodes (`c`, `cp`, `sz`, `sx`); only `sz` and `sx` receive deploys
(:274, :288). Claim both (`identity create --name owner` +
`substrate claim` per directory, since each has its own `--dir`), fix the four
`key = "identity.key"` lines (:48, :87, :127, :155), and add `--as owner` to
the two `svc deploy` calls. `c`/`cp` are coordinator/relay only — claim them
too for uniformity, or leave them unowned and say why in a comment; **leaving
them unowned is fine and cheaper**, since nothing deploys to them.

### 5.6 `roymctl identity certify-instance` (an A0 flow, changed by P0)

`certify_instance` calls `client.instance_identity(service_id)`
([member_identity.rs:113](../../../../apps/roymctl/src/commands/member_identity.rs#L113)),
which is gated on `orchestrator/status`
([orchestration.rs:545-556](../../../../crates/control_plane/src/service/orchestration.rs#L545)).
After Phase 3 it is **denied on an unowned substrate** and needs
`--as <controller>` (or a `--ucan` grant covering that app) on a claimed one.

No code change. What is required:

- `IdentityCommands::CertifyInstance`'s doc comment
  ([identity.rs:75-106](../../../../apps/roymctl/src/commands/identity.rs#L75))
  gains a line: this queries the substrate and so needs an operator identity
  the substrate authorizes.
- `docs/developer-guide.md`'s §5.1 delegation runbook (~:485-515) shows the
  flag.
- The two e2e uses are already covered by §5.2's harness edits (both run as
  the `operator` identity that becomes the node owner) — worth an inline
  comment there saying *why* that identity must be the owner, so a later
  refactor does not quietly split them.

### 5.7 `tests/perf` (§0.3a)

**Harness** — `tests/perf/src/orchestrator.rs`. `TestEnvironment::new`
([:59-72](../../../../tests/perf/src/orchestrator.rs#L59)) already generates
the node key, writes it to `_key_file`, and keeps it as
`pub substrate_identity: Identity` — all before `start_substrate` — so the
agreement can be minted from what the struct already holds:

```
// TestEnvironment::new, after substrate_identity is generated
let owner = Identity::generate()?;
let owner_key = owner.to_bytes();                              // [u8; 32], see below
let agreement = ControllerAgreement::issue(&substrate_identity, &owner, None)?;
let agreement_file = NamedTempFile::new()?;
fs::write(agreement_file.path(), serde_json::to_string(&agreement)?)?;
// held on the struct like `_key_file`, so the temp file outlives the child

// start_substrate
Command::new(bin)
    .arg("run")
    .arg("--key").arg(self._key_file.path())
    .arg("--agreement").arg(agreement_file.path())     // main.rs:56-58, already exists
    ...
```

Expose `pub owner_key: [u8; 32]` on `TestEnvironment`, not an `Identity`:
`Identity` is deliberately not `Clone`
([keys.rs:89](../../../../crates/identity/src/keys.rs#L89)), and the soak
scenario needs the value inside a spawned task. Reconstruct per client with
`Identity::from_bytes` — the pattern `client_gateway`'s `GatewayState` and
`federated_fdae_e2e.rs` already use.

**Scenarios — swap exactly five clients, and leave six alone.** All eleven are
`SyneroymClient::new`, so a mechanical find-and-replace breaks the suite in a
different way.

| Swap (targets `env.substrate_did` → reaches `orchestrator`) | Leave (targets a deployed service) |
|---|---|
| `tcp_proxy_latency.rs:54` | `wasm_latency.rs:187` |
| `concurrency.rs:50` | `concurrency.rs:86`, `:577` |
| `wasm_latency.rs:151` | `soak.rs:126`, `:331`, `:405` |
| `soak.rs:87` | |
| **`soak.rs:276`** | |

`soak.rs:276` is the one the original sweep missed: it is built from
`substrate_did_clone` **inside the deploy-churn loop's spawned task**, deploys
at `:288`, and is the only site in `tests/perf` that calls `undeploy` (`:324`,
`:364`). Without it `mise run bench:soak` still fails after every other fix —
and `bench:soak` shares the harness §6 leans on to catch exactly this class of
breakage. The churn loop needs `owner_key` moved into the task alongside the
existing `substrate_did_clone`/`registry_url_clone`; the owner's
`substrate/admin` entails `orchestrator/undeploy`, so the undeploy calls need
no separate grant.

The six "leave" clients connect to a deployed service's own `service_id`, not
the substrate's, and reach a WASM/TCP service rather than a native interface —
no capability is checked on that path, and giving them the owner identity
would misrepresent what the benchmark measures.

### 5.8 Not affected — verified, so nobody re-checks

- `crates/smoke-tests` — registry publish/lookup only; no `orchestrator` or
  `security` call.
- `crates/sandbox_wasm/tests/*`, `crates/router/tests/{deploy_grant,service_ownership,ucan_context,native_dispatch_identity}.rs`
  — hand-built `CallerContext`s, never routed through `build_caller`, so **no
  assertion changes**. They still need the naming/comment corrections
  enumerated in §3.3 (including the `service_ownership.rs:354` test *name*),
  which is bookkeeping, not behavior.
- `crates/coordinator_iroh/tests/*` — no deploy.
- `SyneroymClient::wait_for_ready` — the empty-`service_id` `readyz` liveness
  ping is deliberately ungated
  ([orchestration.rs:497-510](../../../../crates/control_plane/src/service/orchestration.rs#L497)),
  so `connect()` still works for an unauthorized caller. **This is the single
  most important thing not to break.**

---

## §6 — Tests

### Unit

| Test | File | Proves |
|---|---|---|
| `issue_produces_a_mutually_signed_agreement_that_verifies` | `crates/identity/src/substrate.rs` | round trip: `issue` → `SubstrateIdentityState::init` → `Verified` |
| `issue_rejects_a_self_owned_agreement` | same | §0.1 / D-P0-4 |
| `an_agreement_naming_another_node_is_not_verified` | same | `controlled != substrate_did` path still holds for a minted artifact |
| `an_expired_agreement_is_not_verified` | same | `issue(.., Some(0))` then `init` → `Unverified` (and `Err` when `require_agreement`) |
| `an_agreement_with_an_unparseable_expiry_is_not_verified` | same | §0.9 fail-open fix |
| `an_agreement_with_an_unknown_type_is_not_verified` | same | D-P0-7 |
| `a_tampered_agreement_field_invalidates_both_proofs` | same | canonical-payload coverage (flip `controller`, re-serialize, expect `Unverified`) |
| `an_unowned_substrate_grants_no_node_wide_capability` | `crates/router/src/route_handler/io.rs` | **matrix row 17**, Phase 3 |
| `the_owner_still_receives_substrate_admin` | same (extend the existing owned-substrate test) | Phase 3 did not break ownership |
| `security_is_denied_without_substrate_admin` | `crates/control_plane/src/service.rs` | **matrix row 16**; assert the `-32010` code, not the message |
| `security_is_allowed_for_a_substrate_admin_caller` | same | replaces the caller in `test_security_dispatch_returns_sdk_statuses` |
| `a_discovered_agreement_is_loaded_without_config` | `crates/substrate/src/identity.rs` | D-P0-5: write `agreement.json` into a temp `app_data_dir`, `setup_substrate_identity` with `agreement: None` → `Verified` |
| `a_malformed_discovered_agreement_fails_the_boot` | same | D-P0-6 |

### CLI

| Test | File |
|---|---|
| `claim_writes_a_verifiable_agreement` — `issue` + write + read back + `init` → `Verified` | `apps/roymctl/src/commands/substrate.rs` `#[cfg(test)]` |
| `claim_refuses_to_overwrite_without_force` | same |
| `claim_reports_a_missing_substrate_key_with_the_init_hint` | same |

Follow `commands.rs`'s `client_for_rejects_ucan_without_as` precedent: factor
the body into a testable `fn claim(dir, controller, substrate_key, out, expires_days, force) -> Result<ControllerAgreement>`
that the clap arm calls, so the tests do not shell out.

### Integration / e2e

| Test | File | Proves |
|---|---|---|
| `an_unowned_substrate_rejects_a_deploy` | new, or extend `crates/router/tests/deploy_grant.rs` | matrix row 17 at the `ControlPlaneService` level (caller with zero capabilities → `deploy` denied). Note `deploy_grant.rs`'s test 1 already covers "no grant → denied"; the *new* content is that this is now the unowned case, so prefer extending its module doc over adding a duplicate test. |
| `a_claimed_substrate_admits_its_controller_and_denies_everyone_else` | new `crates/substrate/tests/substrate_ownership_e2e.rs` | the full path over a real substrate: `ControllerAgreement::issue` → written to `app_data_dir` → boot → owner client deploys and injects a KEK; a second, unrelated identity is denied both. This is the only test that exercises discovery, the handshake, and both gates together. |
| Existing e2e (Playwright, both configs) | §5.4/§5.5 | the operator-facing claim flow actually works end to end |

### Gates

The standard four, plus two that are **not** optional for this slice because
the standard four cannot see the code P0 breaks:

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
mise run test:e2e
mise run bench:latency
mise run test:smoke
```

- `mise run test:e2e` covers **both** Playwright configs already —
  `crates/substrate/tests/e2e/package.json:6` runs `playwright test &&
  playwright test -c playwright-multihop.config.ts`. No separate invocation.
- **`mise run bench:latency` is a required gate here** (§0.3a). `tests/perf`
  is not in the workspace test path, so `cargo test --workspace` never runs
  it and it fails only when a benchmark does. Running one perf scenario is
  enough to prove the §5.7 fix; `bench:concurrency`/`bench:soak` share the
  same harness. (`bench:micro` is Criterion-only and does not start a
  substrate — it proves nothing here.)
- `mise run test:smoke` is expected to be unaffected (§5.8), but §5.8 is a
  claim from reading the code, not a proof. Run it.

---

## §7 — Phase order and mergeability

One PR. The three phases are separable in *review* but not in *merge*:
Phase 2 alone denies KEK ops to everyone (B7 F3.1's "functional regression,
not a tightening"), and Phase 3 alone bricks a substrate permanently.

Suggested commit sequence inside the PR:

1. `feat(identity): mint controller agreements` — §2.2, §2.3, §2.4, §2.5, §2.6
   and their unit/CLI tests. Green on its own.
2. `feat(control-plane): gate the security interface on substrate/admin` —
   §3, plus §3.4's test fix. Green on its own (the six harnesses do not touch
   `security`… **except the 12 `inject_kek` sites**, so this commit needs
   §5.1/§5.2/§5.3's harness ownership changes to be green — fold them here,
   not into commit 3).
3. `feat(router): fail closed on an unowned substrate` — §4, §4.3, §5.4, §5.5,
   **§5.7 (`tests/perf`)**, and the §3.3 comment sweep.
4. `docs: claiming a substrate` — §2.6's three developer-guide edits, the two
   config samples, and §5.6's `certify-instance` note. Separable because it
   changes no code; **not** droppable, since every orchestrator example in the
   guide stops working at commit 3.

If commit 2's harness work turns out to be the bulk of the diff, splitting it
into its own `test(substrate): give integration harnesses an owner identity`
commit ahead of 2 is better — it is a no-op change under the current posture
and reviews cleanly.

---

## §8 — Docs and backlog

**Update:**

- `docs/planning/milestones/M05A-app-supervisor/status.md` — P0 row → Complete
  with evidence; a `## P0 — Verification evidence` section in the A0/A1/A2
  house style; note that A3's gate is now clear.
- `docs/planning/milestones/M05A-app-supervisor/task.md` — fix the two stale
  line citations (§0.7); record §0.8 (the tool is local-only) in P0's scope
  note, since it constrains A3's provisioning story.
- `docs/planning/traceability-matrix.md` — the `[FND-IAM]`
  (`ControllerAgreement` creation tool) row → **Complete**; the `[LFC-MGT]`
  row's "**Gated on** the `ControllerAgreement` creation tool row below"
  clause → ungated.
- `docs/planning/deferred-backlog.md`:
  - §11 *Open in-code markers* — **delete** the `control_plane/src/service.rs:256`
    row (the `TODO(M04B/FDAE)` marker is removed by §3.2).
  - §3 — move *"`ControllerAgreement` creation tool — pulled forward"* to
    *Recently resolved*.
  - §3 — move *"`resolve_relation`'s A1/A2 fork is defeated on an unowned
    substrate"* to *Recently resolved* (§0.6), naming P0 as the cause.
  - §11 — the two `TODO(B7b / post-B7)` markers in `io.rs`/`dispatch.rs` are
    **narrowed**, not removed; update their text to say `security` is done and
    only the five data interfaces remain.

**Add (new backlog rows):**

| Row | Theme | Reason | Target |
|---|---|---|---|
| No remote substrate claim | §3 Access control | The tool needs the node's private key, so ownership is established per-host. Provisioning N substrates for A3 means N host visits or out-of-band `agreement.json` distribution. A TOFU claim endpoint is the alternative and was not built. | TBD |
| Claiming requires a substrate restart | §8 Node lifecycle | `setup_substrate_identity` runs once at boot; there is no reload path (unlike TLS's SIGUSR1). Claiming a *running* node needs a restart. | TBD |
| Client gateway presents the node DID, which now holds nothing | §7 Gateway | `gateway.rs:34-38`'s `TODO(post-B0)`. Inert today (the gateway proxies only to deployed services), but it is routing-accidental, not guaranteed. | TBD |
| `Capability::grants` wildcards *any* bare `substrate:` resource | §3 Access control | `grants` short-circuits on `is_substrate_scope` without comparing node DIDs ([capability.rs:186-191](../../../../crates/ucan/src/capability.rs#L186)); a `substrate:<other-node>` + `substrate/admin` capability would pass the new `security` gate. Unreachable today — `build_caller` only ever issues capabilities on this node, and the UCAN root check applies `resource_is_local` — so this is defense-in-depth, not a live hole. | TBD |
| Ownership transfer / revocation has no story | §3 Access control | `claim --force` replaces the agreement on the next restart, and that is the entire mechanism. No revocation, no multi-owner (F12, already tracked), no audit. | M05/TBD |
| **A supervisor needing vault writes must hold `substrate/admin` on every managed substrate** | §3 Access control | D-P0-8 makes all three `security` methods node-owner-only, including the per-service `set-secret`. A5's supervisor that provisions secrets for the services it manages would therefore need node-wide `substrate/admin` on each target — which entails *everything* on those nodes, directly weakening failure-matrix **row 14**'s claim that a compromised supervisor's blast radius is "bounded to the members it manages". The fix is a per-service `set-secret` gate (`substrate:<node>/app/<svc>`), which P0 deliberately does not build. **Evaluate before A5 commits to its custody model**, not after. | **M05A A5** |

---

## §9 — Questions for the requester

These change what gets built; the plan takes a position on each but they are
genuinely the requester's call.

1. **D-P0-10 — no escape hatch.** Confirm there should be no
   `[iam].allow_unowned_deploy` dev flag. The plan says no (a flag makes the
   flip untested wherever it is set); the cost is §5's nine-harness sweep (including `tests/perf`, §0.3a).
2. **D-P0-5/D-P0-6 — implicit `agreement.json` discovery.** It removes a
   silent-failure step, at the cost of the substrate reading a file nobody
   configured. Confirm, or require an explicit `[identity].agreement`.
3. **§5.5 — claim the multi-hop `c`/`cp` nodes or not.** The plan leaves them
   unowned (nothing deploys to them). Uniformity argues the other way.
4. **§0.9 — the two verification tightenings** (`type` enforcement, expiry
   parse) are adjacent-but-not-required scope. Take them here, or spin them
   out?
5. **Does A3 need remote claim?** §0.8's limitation is acceptable for P0 but
   shapes A3's substrate-inventory provisioning story. If A3 assumes an
   operator can claim a substrate they have no shell on, that is a P0 scope
   change, not an A3 one.

---

## §10 — What this hands A3 and A5

**A3 is not blocked on a second missing tool.** A3's substrate inventory holds
"the deploy capability held on each substrate", and its producer already
exists —
[`IdentityCommands::IssueGrant`](../../../../apps/roymctl/src/commands/identity.rs#L209):

```bash
roymctl identity issue-grant --from <owner> --to <supervisor-did> \
  --can orchestrator/deploy --with 'substrate:<node>/app/*' --expires-days 30
```

`--expires-days` is a required `u64`, not an `Option`
([identity.rs:67-69](../../../../apps/roymctl/src/commands/identity.rs#L67)),
and the resource is quoted so the shell does not expand `*`. The grant is
verified at ingress by `build_caller`'s `is_root` and, after Phase 3, the only
route by which a non-owner reaches `orchestrator/*`. P0 makes that grant
*meaningful* rather than redundant. The only per-substrate manual step A3
inherits is §0.8's local claim.

**A5 inherits one unresolved tension**, recorded as a backlog row in §8: under
D-P0-8 a supervisor that writes secrets on its managed substrates needs
node-wide `substrate/admin` on each, which conflicts with failure-matrix row
14. Decide it when A5 chooses its custody model, not by discovering it in
implementation.
