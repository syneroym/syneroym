# M06C Slice C4 — Identity, Profile, Contacts, and Safety: Implementation Plan

> **Scope.** [task.md](task.md)'s **C4** row — R1 rows 1 and 6: a
> device-bound person identity with encrypted backup and an import that
> reproduces it on a clean node; Profile & Contacts, including the
> person→conversation-address mapping Gap 5 forces; and block, report,
> per-sender contact rate limits, publication limits, and policy
> disclosure (the `[PRD-SAF]` backlog row). Block is Roym-side and honestly
> described (`D-06C-8`). Gate: **C3**.
>
> **C4 also owes the three C3 residuals that were targeted at it by name**
> ([deferred-backlog.md](../../deferred-backlog.md) §3, rows added by C3):
> the certificate lifecycle (`D-C3-13`), the internal-caller binding
> (`D-C3-12(b)`), and the external-caller gate on `signing`
> (`D-C3-12(a)`) — plus §7's *"fine-grained session authorization gating
> on `/rpc` routes"* row, which C2 targeted at C4.
>
> **Read §18 first if you are executing this plan.** Six claims in the
> input documents do not hold against the tree. Three of them
> (**A**, **B**, **J**) change what gets built.
>
> **Planning identifiers appear in this document and must not appear in
> the code it describes.** AGENTS.md forbids slice and milestone ids in
> comments and doc comments, not only in names. Every code block below is
> written with that already applied — the reasoning is carried by the
> prose around it, and a comment states the constraint, never the slice
> that introduced it. §15 item 9 checks comments, not just names.

---

## §0 What C1 / C1.1 / C2 / C3 handed C4, and what is missing

| Handed over | Where |
|---|---|
| Six service crates that build both ways, with one `invoke(request) -> result<string,string>` + `status()` surface each | `crates/roym_{web,conversation,profile,catalog,transaction,directory}` |
| A method-prefix routing table already carrying `profile.`, `contacts.`, `block.` | `crates/roym_core/src/router.rs:12` |
| The person's DID, verified, reaching `web` — and only `web` | `crates/router/src/route_handler/http.rs` (`resolve_effective_session_caller`) → `HttpRequest.caller` |
| A signing host interface that never returns key material, and refuses a draft it cannot parse | `crates/wit_interfaces/wit/signing/signing.wit`, `crates/core/src/record_signer.rs` |
| A canonical envelope + verifier that compiles for `wasm32-wasip2` and takes no `Identity` | `crates/signed_record/` |
| A CLI that mints one `record-signing` certificate for one service | `roymctl identity certify-signing` (`apps/roymctl/src/commands/identity.rs:115`), `syneroym_sdk::deploy::certify_record_signing` (`crates/sdk/src/deploy.rs:625`) |
| A Hub shell with a working `delegated-key` login and a card renderer | `crates/roym_web/ui/src/{main.ts,session/login.ts,cards/}` |
| A raw, unencrypted person key file at `<dir>/identities/<name>.key`, mode 0600 | `Identity::save_to_path` (`crates/identity/src/keys.rs:151`), `roymctl identity create` |
| A gateway-host printer for a service DID | `roymctl alias <service-did>` → `util::generate_service_host` (`apps/roymctl/src/commands.rs:257`) |

Missing, and C4's to build:

1. **No product state anywhere.** `roym_profile::app::invoke` answers
   `profile.ping` and nothing else
   (`crates/roym_profile/src/app.rs`). No collection is created by any
   Roym service on either build.
2. **No certificate lifecycle.** `certify-signing` mints one certificate
   for one service, needs the master key file locally, has no storage, no
   renewal, and no route for a browser-only person.
3. **No backup of anything.** `syneroym-data-keystore` has no export;
   `Identity` writes 32 raw bytes; `roymctl` has no backup verb.
4. **No safety primitive.** No block list, no report, no contact ceiling
   above the prekey-bundle limiter in `crates/conversation/src/store.rs`.
5. **No authorization on `/rpc`.** Every `web` route is `public: true`
   and `web` forwards every method it can route, signed in or not.
6. **`signing/sign-record` is reachable by any caller the router admits**
   to a service (C3 `F15`, and `record_signing_e2e.rs` step 8 asserts it).
7. **No content-hash primitive a guest can reach.** `syneroym-signed-record`
   hashes inside `Envelope::record_id` and exports nothing reusable.

---

## §1 Findings from reading the tree

Verified 2026-09-01 against `main` at `64d4eb2`. Each is load-bearing for
a decision in §2.

### F1 — a Roym guest **can** read wall-clock time, and nothing in the tree says so

No Roym world imports `wasi:clocks`, so this looked closed. It is not.
`test-components/miniapp-demo1-wasm` calls `SystemTime::now()`
(`src/lib.rs:208`) and its built component imports
`wasi:clocks/wall-clock@0.2.9` — confirmed with
`wasm-tools component wit` against
`test-components/miniapp-demo1-wasm/target/wasm32-wasip2/release/syneroym_test_miniapp_demo1_wasm.wasm`,
not by inspection of the WIT. `std`'s own imports are merged into the
component type independently of the `generate!` world, and
`p2::add_to_linker_async` (`crates/sandbox_wasm/src/engine.rs:751`)
satisfies them. The currently-built `syneroym_roym_profile.wasm` imports
`monotonic-clock` but **not** `wall-clock`, because nothing calls it yet.

So rate limits and freshness are expressible in guest code with no new
host interface and no world change. **But see F5**: the guest clock and
the host's signing clock are separate, and only the host's is pinnable.

### F2 — dependency resolution is per **app instance**, not per declared `depends_on`

`host_capabilities.rs:1396` resolves `CallTarget::Dependency(name)` through
`LogicalResolver` keyed by `TopologyKey::local(app_instance_id, name)`,
where `app_instance_id` comes from the *caller's* own
`service_app_context` row. `install_app_context`
(`crates/control_plane/src/service/orchestration.rs:672`) registers one
entry per binding into the node-wide `StaticInventory`, and `init_roym`
(`crates/substrate/src/runtime.rs`) does the same for the native build.

Consequence: **any Roym service can already resolve
`Dependency("profile")`, even though only `web` declares `depends_on`.**
That is what will let C5's inbox ask `profile` whether a sender is
blocked with no manifest change. It is also a latent authorization gap —
a declared dependency graph that nothing enforces — and §14 gives it a
backlog row rather than papering over it with manifest entries no code
traverses.

### F3 — `web` is the **local person's** entrypoint only; the wire reaches siblings directly

`web`'s `depends_on` fan-out is driven by a browser on `127.0.0.1` behind
a verified session. A foreign consumer never traverses `web`: it resolves
`catalog`/`conversation`/`directory` by DID and calls `api.invoke` on them
over Iroh (`roym.toml`'s `visibility = "public"`,
`topology_visibility = "open"`). So an authorization rule applied in `web`
covers exactly one ingress and says nothing about the wire-facing one.
This is why `D-C4-4` mounts the certificate verbs on the one service that
is `visibility = "private"` and nowhere else.

### F4 — what a callee actually sees as its caller, precisely

Three different values, and the distinction matters for `D-C4-9`:

| Path | `caller_did` at the callee |
|---|---|
| Guest G proxies to a **different** service | `system:<G's own id>` — `host_capabilities.rs:1453-1455` synthesizes `service_system(&self.component_id)` |
| Guest G proxies to **its own** service (self-proxy) | `G`'s own real caller, forwarded unchanged — the same branch, `if target == self.component_id` |
| A **WASM** callee reached through `ProxyRouter` | `system:<callee's own id>` — `invoke_local` passes `None` for a `WasmChannel` target, and `prepare_wasm_execution` falls back to `service_system(service_id)` (`engine.rs:1378`) |

So for a Roym sibling reached from `web` on the WASM build,
`HostState.caller` is `system:<that sibling's own id>` — produced by the
**no-caller fallback**, not by any self-proxy. All three are
`AuthLevel::System`, so `caller_binding` (`host_capabilities.rs:581`)
yields `CallerBinding::Internal` and
`NodeRecordSigner::sign_record`'s `cert.master_did == caller_did` check
never runs on a sibling call. That is `D-C3-12(b)` verbatim, and it is why
`D-C4-10` does not try to close it with a caller check.

### F5 — the guest clock and the host's signing clock are different domains, and only the host's can be pinned

`NodeRecordSigner` stamps `issued_at_secs` from `RecordClock`
(`crates/core/src/record_signer.rs:56`), which a test can set to
`Fixed(n)`. Guest code reading `SystemTime::now()` (F1) cannot be pinned
from outside the component at all. Three consequences the first draft of
this plan got wrong:

1. A guest that verifies its own freshly signed envelope with
   `VerifyOptions::new(guest_now)` fails `IssuedInFuture` whenever the
   host clock is `Fixed` and ahead of the wall clock —
   `crates/signed_record/src/verify.rs:167` rejects
   `issued_at > now + max_clock_skew_secs` (default 300).
2. A certificate must satisfy both clocks at once: the guest checks it at
   install, the host at sign. `D-C4-6` removes the conflict by giving the
   guest only the check it can make soundly.
3. Any value a guest derives from its own clock differs between two runs,
   so it cannot be asserted equal across builds. `D-C4-12` says what is
   compared instead.

### F6 — `signing/identity` is the right thing to certify against; `resolve-instance-identity` is not

`NodeRecordSigner::identity` derives under `registry.owner_of(service_id)`
(`crates/core/src/record_signer.rs:80`), while
`orchestrator/resolve-instance-identity` derives under the *caller's* DID.
`syneroym_sdk::deploy::certify_record_signing` already compensates by
calling `client.signing_identity(service_id)` and refusing when
`owner_did != master_did` (`crates/sdk/src/deploy.rs:637`). Asking the
*service* for its own signing identity — what C4's enrolment does, over
`profile.signing-status` — removes the mismatch by construction.

### F7 — `create-collection` is idempotent, and indexes are `IF NOT EXISTS`

`crates/data_db/src/sqlite.rs:112` and `:123`. The fixture's
`ensure_collection` (`test-components/dual-build-fixture/src/app.rs:244`)
is the established pattern: call it on first use of every collection,
every invocation. C4 follows it — no `init`/`migrate` export, which the
Roym worlds do not have anyway (`data-layer-import`, not
`data-layer-guest`).

### F8 — the filter DSL supports exactly what the safety rules need

`crates/data_db/src/filter.rs` compiles `$and`/`$or`/`$not`,
`$gt`/`$gte`/`$lt`/`$lte`/`$ne`/`$in`/`$nin`/`$regex`, and bare equality,
against `json_extract(payload, ?)`. So the contact-attempt window query
(`{"sender_key": K, "at_secs": {"$gte": N}}`) is expressible with one
collection and one numeric index — no `execute-ddl`, no raw SQL, which
stay C6's (Gap 7).

### F9 — a roym crate may depend on seven things, and nothing else

`xtask check-roym-deps` (`xtask/src/main.rs:70`) allows
`syneroym-app-host`, `syneroym-roym-core`, `syneroym-signed-record`,
`serde`, `serde_json`, `async-trait`, `thiserror` in `[dependencies]`.
**`sha2` and `z32` are not on that list**, so `roym_core` cannot compute a
content hash today: `syneroym-signed-record` owns both and exports neither
(`Envelope::record_id` hashes internally,
`crates/signed_record/src/envelope.rs:199`). §3.1 adds the missing export
rather than widening the allowlist.

### F10 — `syneroym-identity` already has every crypto dependency an encrypted backup needs but one

`crates/identity/Cargo.toml` carries `hkdf`, `sha2`, `z32`, `getrandom`,
`zeroize`, `ed25519-dalek`. Only `aes-gcm` (already in the workspace's
`[workspace.dependencies]`, root `Cargo.toml:142`) is new. There is **no**
password KDF in the workspace (`argon2`/`scrypt`/`pbkdf2` all absent) —
which makes `D-C4-7`'s recovery-key choice cheap and a passphrase choice a
new dependency plus a parameter-versioning problem.

### F11 — no test reaches `vault/reveal` or `signing/sign-record` externally except C3's own

`grep` for `"vault"` and `reveal` across `crates/*/tests` matches only a
doc comment in `gateway_hostname_e2e.rs`. The only external
`sign-record` caller in the tree is `record_signing_e2e.rs` steps 8 and 9,
which C3 wrote *expecting* this slice to invert step 8. So `D-C4-9`'s
gate has a bounded, known blast radius.

### F12 — `NativeInvocation` carries no origin, so the gate cannot live in the router

`crates/rpc/src/native.rs:182` is `{ interface, method, params, caller }`.
`ProxyRouter::check_native_capability_gate` short-circuits on
`CallOrigin::Guest` (`crates/router/src/proxy.rs:590`) and so never sees
an external call. Adding an origin field would touch ~45 construction
sites across nine files. The gate therefore goes inside
`SynSvcNativeService::dispatch_signing` / `dispatch_vault`, which are
reached **only** by an external caller and by a guest's own same-service
self-proxy — a guest's own host import goes straight to `HostState` and
never passes through them.

### F13 — `roym_core::record` shipped narrower than C3's plan described, and carries a forbidden comment

`crates/roym_core/src/record.rs` has `RECORD_TYPES: &[&str]` (nine names,
no versions) and `is_known_record(&str)`; C3's plan §11.2 specified
`&[(&str, u32)]` and `is_known_record(&str, u32)`, mirroring
`card::CARD_TYPES`. The re-export list is also shorter than planned.
Separately, line 7 reads `/// Known Roym record types (spec D-C3-14).` —
a slice id in a doc comment, which AGENTS.md forbids;
`crates/roym_core/app/roym.toml:15` has the same problem
(`# In Slice C2, ...`). These are the only two in the Roym tree. C4
corrects all of it (§3.2, §9).

### F14 — the installation's owner is the **deployer**, and the existing Roym e2e logs in as somebody else

`ControlPlaneService` records `set_owner(service_id, caller.caller_did)`
at deploy (`crates/control_plane/src/service/orchestration.rs:2239`).
In `crates/substrate/tests/roym_app_e2e.rs` the deploy runs through an SDK
client built from `ctx.owner` (`:214-221`) while the person who logs in is
a freshly generated `alice` (`:163-166`, `:313`). The two DIDs differ.

This is not a defect the plan introduces — **C3 already requires it**:
`NodeRecordSigner` derives the signing key under `owner_of(service_id)`,
and `certify_record_signing` refuses when `owner != master`
(`crates/sdk/src/deploy.rs:637`), so only the deployer's master key can
mint a certificate that will be accepted. `D-C3-13` states it as *"whoever
can certify is the DID that deployed the service"*. C4 is where that
becomes a user-visible product constraint, and §11.3 / §12 schedule the
harness change it forces.

`AuthService::login_local` reads the key file **on every login**
(`crates/auth/src/service.rs:344-363`), not at boot, so the harness can
write `ctx.owner`'s key into the person-identities directory *after*
`SubstrateTestContext::setup_with` returns. That is the whole fix.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C4-1** | **For the first release a Roym installation belongs to exactly one person: the DID that deployed it, which `EndpointRegistry::owner_of` records.** Every service learns it from `AppSigning::signing_identity().owner_did`, never from a payload claim. `web` refuses any `Owner`-classified method whose verified session subject is not that DID. Stated in the Hub and in `profile.policy`, not left implicit. | Both facts are host-attested — `owner_did` from the registry, the session subject from the router's own cookie verification — so nothing is trusted from the wire. `F4` says a sibling cannot be told who is asking soundly, and `D-C2-4` retracted the attempt; this is the one identity a sibling can establish for itself. **The person-is-the-deployer half is inherited, not invented** (`F14`): C3's key derivation already makes any other arrangement unable to mint an accepted certificate. Its real cost is that the spec's managed-guild path — a SynOrg hosting a provider's substrate — cannot work, because the host would be the deployer and therefore the person. Named in §14 as a backlog row rather than discovered in C9. |
| **D-C4-2** | **`roym_core` gains a `MethodAuth` column beside the routing table, and `web` enforces it before it forwards. Two values only: `Public` and `Owner`. No default arm — a routable method with no classification fails to compile.** | Closes §7's C2-targeted backlog row at the only layer that holds the caller (`F3`). It is admission, not business logic, and it stays a *lookup*: the reasoning `D-C2-3` used to keep the routing table a table. A default arm would make "which methods are open?" a decision made by omission. |
| **D-C4-3** | **C4 adds exactly one record type, `profile`, to the spec's nine, and edits the spec's Records table to carry it.** Signed by the person, subject the person's own master DID, payload `{display_name, about?, conversation_address, locale?}`. An edit is a new record with `supersedes`; nothing is rewritten. | Two independent reasons. (a) **Gap 5**: a first contact from an unknown DID must carry something the receiver verifies itself, and a listing (C5) must embed the provider's conversation address under the provider's own signature. `D-C4-17` gives that a C4 code path so the claim is not aspirational. (b) **C4 cannot otherwise prove the certificate lifecycle it is required to deliver** — shipping the mechanism with no product flow through it is the failure `D-06B-3`/`D-06C-10` exist to prevent. **Alternative if a reviewer refuses the type:** move the certificate lifecycle to C5. "Ship it unexercised in C4" is not a third option. |
| **D-C4-4** | **A `record-signing` certificate is app state: one per service, in that service's own DEK-encrypted `data-layer`, installed through an authenticated verb, replaced by re-running enrolment. The shared implementation lives in `roym_core::signing`; in C4 it is mounted on `profile` alone.** | `D-C3-4` keeps certificates out of *substrate* state, and every reason it gives (no new `EndpointStorage` triple, no new table, no boot replay, no single-person lock-in) is about the substrate, not about app state. The alternative — the client sends the certificate on every write — puts a long-lived bearer artifact in a browser and on the wire on every call. Storing it widens nothing: the certificate is still a *parameter* to `sign_record`, chosen by the guest's own code. **Mounted on `profile` only**, because `profile` is the one service declared `visibility = "private"`; `catalog`, `conversation`, and `transaction` are `public` and `directory` is additionally `topology_visibility = "open"`, so mounting there would make `install-signing-certificate` the first wire-reachable *write* verb on Roym's public services, guarded by nothing but its own payload validation (`F3`). The handler is shared so C5/C6 mount it in one line each, by which time wire-side authorization is theirs to own. |
| **D-C4-5** | **Enrolment asks the service for its own signing identity, not the substrate's orchestrator.** `profile.signing-status` returns `{signing_did, pubkey_hex, owner_did, certificate}` straight from `AppSigning::signing_identity()`; `roymctl roym enrol-signing` mints against `signing_did` and posts it to `profile.install-signing-certificate`. | `F6`: `resolve-instance-identity` derives under the caller's DID and the signer derives under `owner_of`, so the two can disagree — `D-C3-13` named it as a constraint to live with. Asking the signer removes the disagreement by construction, and the CLI never needs a service DID. |
| **D-C4-6** | **The guest checks a certificate only for what it can check soundly: shape, scope, key, master, and *already expired*. It does not check "not yet valid".** The host re-checks everything, including the lower bound, on every sign. | `F5(2)`: the guest clock and the host's `RecordClock` are different domains, so a lower-bound check in the guest can refuse a certificate the host would accept, for no safety gain. Expiry is the check with a real consequence (a certificate past its window will be refused at sign time, and telling the person that at enrolment beats telling them at their first record), and it fails in the safe direction. |
| **D-C4-7** | **The person's identity backup is encrypted under a randomly generated 32-byte recovery key, shown once, z-base-32 encoded — not under a passphrase.** HKDF-SHA256 → AES-256-GCM, AAD binding the DID and the header. It lives in `syneroym-identity` behind a non-default `backup` feature, enabled by `roymctl` and nothing else. | `F10`: the workspace has no password KDF, so a passphrase means a new dependency *and* a KDF-parameter version field *and* a weak-passphrase failure mode nothing here can measure. The feature gate keeps `aes-gcm` out of the `wasm32-wasip2` build of `syneroym-signed-record`, which depends on this crate. **Passphrase wrapping is additive later** — a second `kdf` value, refused rather than guessed when unknown, which is `D-06C-1`'s own rule. |
| **D-C4-8** | **The app-data export bundle is versioned and integrity-checked by content hashes, and carries no signature in C4.** `roym_core::backup::{Bundle, BundleManifest, SectionDigest}`; C4 owns `profile`, `contacts`, `blocks`, `reports`. C5 adds conversation sections; C8 adds the transaction sections, the signed manifest, and the top-level composition. | R1 row 1 needs restore-on-a-clean-node; R2's export row (a signed bundle) is C8's by `task.md`'s own slice table. Adding a signature field later costs nothing *precisely because* nothing signs the bundle today — the same argument `D-06C-1` makes for why the version field is expensive later and this one is not. An always-`None` signature field would be an untested field, which `D-C1-10` refuses. |
| **D-C4-9** | **`signing/sign-record` and `vault/reveal` over native dispatch are admitted only for the service's own synthetic `system:<service_id>` identity or the service's recorded owner. `signing/identity` stays open.** Implemented in `SynSvcNativeService::dispatch_signing`/`dispatch_vault`; `record_signing_e2e.rs` step 8's assertion inverts. | `D-C3-12(a)`'s backlog row, targeted here, and its own note that a fix covering `signing` alone while `vault` keeps the identical exposure is not a fix. `signing/identity` stays open because it returns public identifiers only, by its own WIT doc — gating a read of public data buys nothing, and `syneroym_sdk::deploy::certify_record_signing` still reaches it over native dispatch even though `D-C4-5`'s ceremony does not. `F12` puts the check in the dispatcher: the one place reached by exactly the two callers that matter. `F11` bounds the blast radius to one test file. |
| **D-C4-10** | **`D-C3-12(b)` — a `delegated` sign driven by an `Internal` caller — is *not* closed by a caller check, and C4 says so in one place instead of leaving it implied.** What replaces the check is three facts, each independently testable: the certificate is minted only by the master key; it is readable only from that service's own encrypted store; and `web` refuses any state-changing method whose session subject is not the owner. The backlog row is restated with what closed and what did not, not marked resolved. | `F4` says there is no verified DID on the sibling path to check against, and manufacturing one in the guest is what `D-C2-4` retracted as unsound. The honest close is to remove the *reachability*, not to add a check that cannot be sound. A real caller-attested channel stays M6's cross-service caller-identity pass. |
| **D-C4-11** | **No `depends_on` edge is added to the manifest in C4.** `F2`'s hole — a service can resolve a dependency it never declared — gets a backlog row with the trigger *"a binding check enforces declared dependencies"*, and C5 declares `conversation → profile` when it adds the caller. | An earlier draft of this plan added four edges for honesty. That is a declaration nothing traverses, which is the same "untested by construction" problem `D-C1-10` refuses everywhere else, and it would make the router's manifest test assert a graph the code does not walk. Recording the gap is the honest half; declaring edges is not. |
| **D-C4-12** | **Wall-clock time is read in exactly one function, `roym_core::clock::now_secs()`, called only at the outermost verb boundary; every rule below it takes `now_secs: u64`. Cross-build equality is asserted on the *signed* artifact, whose timestamp is the host's and therefore pinnable, and on every unsigned artifact **after** volatile fields are normalized out.** The parity suite gains one `strip_volatile` helper, and the volatile fields are asserted separately for presence and plausibility. | `F5`. Passing `now_secs` down makes rules unit-testable; it does not make verbs comparable, because both builds still read a real clock that has moved between the two runs. The alternative — an injectable guest clock through `app-config` — means shipping a production config key whose only purpose is to lie about the time, which is a worse thing to ship than a test helper. `report_id` is made clock-free (`D-C4-14`) rather than normalized, because content-derived identity is worth having on its own. |
| **D-C4-13** | **Block is a Roym-side tombstone and the product says so in the words `D-06C-8` fixed.** C4 ships the list, the decision function, and the disclosure text; **C5 ships the enforcement point**, because Roym's own inbox does not exist until C5 builds it. | `D-06C-8`, and the host's own WIT: `on-message` is a notification delivered after the message is durably stored. R1 row 6's acceptance test therefore closes in C5 — flagged in §18 **C** rather than quietly failed. |
| **D-C4-14** | **`report.` is a new method prefix on Profile & Contacts; a report has two reachable states, `recorded` and `withdrawn`; and `report_id` is derived from the report's content alone, with no timestamp in it.** The states a SynOrg produces (`sent`, `acknowledged`, `closed`) are not defined here at all. | The spec's Profile API column lists no report verbs, so this is an addition the spec owes an edit for. Defining only reachable states is `D-C1-10`'s discipline. Keeping the timestamp out of the id is what makes filing the same report twice converge on one row instead of accumulating one per second, and it is what lets the parity suite compare ids directly (`D-C4-12`). |
| **D-C4-15** | **Publication limits ship as a pure rule in `roym_core::safety` with no caller in C4.** `admit_publication(prior_secs, limits, now_secs)` plus its defaults and unit tests. C5 (catalog-side) and C6 (directory-side) call it. | Publication does not exist until C5/C6, so an enforcement point here would be written against no producer. The rule is what `[PRD-SAF]` asks C4 to fix once; the alternative is two limiters, written twice, differing. A deliberate, named exception to `D-C1-10`, justified because it is pure arithmetic fully covered by unit tests, with no interface and no state. |
| **D-C4-16** | **Contact rate limits are per-recipient settings, not constants.** `contacts.limits` reads them, `contacts.set-limits` writes them, and `roym_core::safety` supplies the defaults and takes the values as a parameter. | The spec's Safety section says *"rate-limited per sender **and controllable by the recipient**"*. An earlier draft shipped constants and would have failed that clause silently. The cost is one collection row and one parameter. Publication limits get the same shape for symmetry, with no writer in C4 (`D-C4-15`). |
| **D-C4-17** | **`contacts.upsert` accepts an optional `profile_envelope`. When present it is verified against the named person, and the contact's address and display name come from the verified record rather than from what the person typed.** `profile.get { person_did }` reads the same store. | Without a writer for a peer's `profile` record, `ContactRow.from_profile_record` is always `None`, the Hub's verified/unverified distinction cannot be set up, and `D-C4-3`(a)'s rationale has no code path. C4's only source for a peer envelope is the person supplying it out of band — which is exactly the "direct link or referral, no directory anywhere" path `D-06C-6a` requires to work. C5's inbox becomes the automatic source. |

---

## §3 `syneroym-signed-record` and `syneroym-roym-core`

### 3.1 `crates/signed_record/src/lib.rs` — three new exports (`F9`)

```diff
 pub use envelope::{
-    DraftError, ENVELOPE_VERSION, Envelope, EnvelopeError, MAX_PAYLOAD_BYTES, MAX_PAYLOAD_DEPTH,
-    MAX_RECORD_TYPE_LEN, MAX_SUBJECT_LEN, RECORD_ID_PREFIX, RecordDraft,
+    DraftError, ENVELOPE_VERSION, Envelope, EnvelopeError, MAX_PAYLOAD_BYTES, MAX_PAYLOAD_DEPTH,
+    MAX_RECORD_TYPE_LEN, MAX_SUBJECT_LEN, RECORD_ID_PREFIX, RecordDraft, content_digest,
 };
-pub use syneroym_identity::delegation::SCOPE_RECORD_SIGNING;
+/// The delegation certificate and the canonicalizer, re-exported so a
+/// component checks a certificate, and hashes a document, with the same
+/// code the host runs. Nothing here can produce a signature: `issue`
+/// takes an `Identity`, and this crate exposes no way to build one.
+pub use syneroym_identity::{
+    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
+    substrate::canonicalize_json_value,
+};
 pub use verify::{ … unchanged … };
```

New in `crates/signed_record/src/envelope.rs`:

```rust
/// z-base-32 SHA-256 over the canonical bytes of `value`. The one content
/// hash this product has: `Envelope::record_id` is this function applied
/// to an envelope, and every other content-addressed identifier is this
/// function applied to something else. `prefix` is prepended verbatim so
/// two families of id cannot collide.
///
/// The canonical bytes are key-sorted `serde_json`, not full RFC 8785 --
/// which is only reproducible for integers, so a value containing a
/// non-integer number hashes differently on a producer that round-trips
/// floats differently. `RecordDraft::validate` refuses those in a payload
/// for exactly this reason; a caller hashing something else must apply
/// the same rule itself.
pub fn content_digest(prefix: &str, value: &Value) -> Result<String, EnvelopeError> {
    let bytes = serde_json::to_vec(&substrate::canonicalize_json_value(value))
        .map_err(|e| EnvelopeError::Json(e.to_string()))?;
    Ok(format!("{prefix}{}", z32::encode(&Sha256::digest(&bytes))))
}
```

`Envelope::record_id` is refactored to
`content_digest(RECORD_ID_PREFIX, &serde_json::to_value(self)?)` — one
definition, and its existing tests (`record_id_is_stable_across_serialize_deserialize`,
`record_id_changes_when_any_field_changes`) keep passing unchanged, which
is the check that the refactor is behaviour-preserving.

> **Verify before going further:**
> `cargo build -p syneroym-signed-record --target wasm32-wasip2`.
> The `DelegationCertificate` re-export widens what the linker keeps;
> C3's `F10` proved `syneroym-identity` itself compiles there.

### 3.2 `crates/roym_core/src/record.rs` — correct the shape, add the type, drop the slice id

```rust
//! Roym's signed record vocabulary, and the envelope re-exports the
//! product reads it through.

pub use syneroym_signed_record::{
    DelegationCertificate, EmptyRevocations, Envelope, RecordDraft, RevocationCheck,
    RevocationSet, RevocationSource, RevocationStatus, SCOPE_RECORD_SIGNING, VerifiedRecord,
    VerifyError, VerifyOptions, content_digest, verify, verify_json,
};

/// Every record type this product produces, and the version each is
/// produced at today. Fixed, like the card table: a record of an unlisted
/// type, or a listed type at an unlisted version, is not understood
/// rather than guessed.
///
/// `profile` proves who published a person's card and the conversation
/// address they claim. It is how a stranger reached by direct link gets
/// an address they can attribute, with no directory involved.
pub const RECORD_TYPES: &[(&str, u32)] = &[
    ("profile", 1),
    ("listing", 1),
    ("membership-credential", 1),
    ("revocation", 1),
    ("request", 1),
    ("quote", 1),
    ("agreement-receipt", 1),
    ("payment-acknowledgement", 1),
    ("fulfilment-receipt", 1),
    ("moderation-decision", 1),
];

pub const RECORD_PROFILE: &str = "profile";

pub fn is_known_record(record_type: &str, version: u32) -> bool {
    RECORD_TYPES.iter().any(|&(t, v)| t == record_type && v == version)
}
```

**Call sites:** the only caller today is this file's own test, which
becomes a two-argument call.

### 3.3 `crates/roym_core/src/clock.rs` — new file

```rust
//! The one wall-clock read in the product.
//!
//! On `wasm32-wasip2` this lowers to `wasi:clocks/wall-clock`, which the
//! sandbox linker provides; natively it is the process clock. Identical
//! call, identical value source, both builds.
//!
//! Call it at a verb boundary and pass the result down. This clock is
//! *not* the clock the substrate stamps a signed record with: that one
//! belongs to the host and a test can pin it, and this one cannot be
//! pinned from outside a component at all. Anything that must be
//! reproducible across two runs has to derive from the host's stamp or
//! from content, never from here.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix seconds. A clock before the epoch is impossible on both targets;
/// it saturates to 0 rather than panicking inside a guest.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
```

### 3.4 `crates/roym_core/src/person.rs` — new file (Gap 5)

```rust
//! Where to reach a person. The conversation interface addresses a
//! *service*, never a person, and person-to-substrate resolution needs a
//! Primary Substrate designation that is unbuilt. So the mapping is this
//! product's own, and this is its one definition: Profile & Contacts
//! stores it, a `profile` record signs it, and a listing embeds it.

use serde::{Deserialize, Serialize};

pub const MAX_DISPLAY_NAME_LEN: usize = 128;
pub const MAX_ABOUT_LEN: usize = 1024;
pub const MAX_ADDRESS_LEN: usize = 256;

/// The `profile` record's payload at version 1. Every field is a value to
/// display, never markup: the Hub inserts these as text nodes only, the
/// same rule that governs every value a card carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePayload {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// The routing service id `open-direct` takes -- this person's own
    /// Conversation service. Signed as part of the profile, so a stranger
    /// who verifies the profile has an address they can attribute.
    pub conversation_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersonError {
    #[error("display name is empty or over {MAX_DISPLAY_NAME_LEN} bytes")]
    DisplayName,
    #[error("about is over {MAX_ABOUT_LEN} bytes")]
    About,
    #[error("conversation address is empty or over {MAX_ADDRESS_LEN} bytes")]
    Address,
    #[error("'{0}' is not a did:key")]
    NotADid(String),
}

impl ProfilePayload {
    pub fn validate(&self) -> Result<(), PersonError> { … }
}

/// `did:key:` plus a non-empty remainder. Deliberately not a multicodec
/// parse: this crate has no crypto, and a DID that passes here but fails
/// key resolution fails loudly at the host boundary, which is where the
/// real check belongs.
pub fn is_did_key(s: &str) -> bool {
    s.strip_prefix("did:key:").is_some_and(|r| !r.is_empty())
}
```

`crates/roym_core/Cargo.toml` gains `thiserror.workspace = true` under
`[dependencies]` (already on `F9`'s allowlist).

### 3.5 `crates/roym_core/src/safety.rs` — new file, pure rules (`D-C4-16`)

```rust
//! The safety rules, as arithmetic. No host, no clock, no storage:
//! everything takes its limits and its `now_secs` as parameters, so every
//! rule is a unit test away from proven and both builds run the identical
//! function.

use serde::{Deserialize, Serialize};

/// The recipient's own ceiling on unsolicited first contact. Settable,
/// because the requirement is "rate-limited per sender **and controllable
/// by the recipient**" -- a constant would meet half of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactLimits {
    pub window_secs: u64,
    pub max_per_window: u32,
}

impl Default for ContactLimits {
    fn default() -> Self {
        Self { window_secs: 24 * 60 * 60, max_per_window: 3 }
    }
}

impl ContactLimits {
    pub const MIN_WINDOW_SECS: u64 = 60;
    pub const MAX_WINDOW_SECS: u64 = 30 * 24 * 60 * 60;
    pub const MAX_PER_WINDOW_CEILING: u32 = 1000;
    /// A recipient may loosen or tighten, within bounds a mistyped value
    /// cannot escape. `max_per_window == 0` is allowed and means "no
    /// unsolicited first contact at all".
    pub fn validate(&self) -> Result<(), LimitsError> { … }
}

/// Same shape for listing publication. No writer in this slice: the
/// producer is the catalog's publish path and the directory's admission,
/// neither of which exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationLimits { pub window_secs: u64, pub max_per_window: u32 }
impl Default for PublicationLimits {
    fn default() -> Self { Self { window_secs: 24 * 60 * 60, max_per_window: 20 } }
}

/// Never a bare `bool`: a refusal must carry why, because the refusal has
/// to be visible to the sender, and "blocked" and "too many, try later"
/// are different things to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Allow,
    Blocked,
    RateLimited { retry_after_secs: u64 },
}

/// `attempts_secs` is every prior first-contact attempt by this sender,
/// in any order. Only those inside the window count.
pub fn admit_first_contact(
    blocked: bool,
    attempts_secs: &[u64],
    limits: &ContactLimits,
    now_secs: u64,
) -> Admission {
    if blocked {
        return Admission::Blocked;
    }
    if limits.max_per_window == 0 {
        return Admission::RateLimited { retry_after_secs: limits.window_secs };
    }
    let floor = now_secs.saturating_sub(limits.window_secs);
    let mut inside: Vec<u64> = attempts_secs.iter().copied().filter(|t| *t > floor).collect();
    if inside.len() < limits.max_per_window as usize {
        return Admission::Allow;
    }
    inside.sort_unstable();
    // The oldest attempt still inside the window is the one that must age
    // out before another is admitted.
    let oldest = inside[inside.len() - limits.max_per_window as usize];
    Admission::RateLimited {
        retry_after_secs: (oldest + limits.window_secs).saturating_sub(now_secs).max(1),
    }
}

pub fn admit_publication(
    prior_secs: &[u64],
    limits: &PublicationLimits,
    now_secs: u64,
) -> Admission { … }
```

Unit tests: empty history allows; exactly at the ceiling refuses; an
attempt one second outside the window does not count; `retry_after_secs`
is never 0; `blocked` beats a clean history; `max_per_window: 0` refuses
everything; `validate` rejects a window below the floor and above the
ceiling.

### 3.6 `crates/roym_core/src/backup.rs` — new file (`D-C4-8`)

```rust
pub const BUNDLE_VERSION: u32 = 1;
pub const SECTION_PROFILE: &str = "profile";
pub const SECTION_CONTACTS: &str = "contacts";
pub const SECTION_BLOCKS: &str = "blocks";
pub const SECTION_REPORTS: &str = "reports";
/// The digest prefix, so a section digest can never be mistaken for a
/// record id or a report id.
pub const SECTION_DIGEST_PREFIX: &str = "sec_";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionDigest {
    /// The section's own schema version, chosen by whichever service
    /// produced it. No serde default: a section with no version fails to
    /// parse rather than assuming one.
    pub schema_version: u32,
    pub record_count: u64,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub bundle_version: u32,
    pub produced_at_secs: u64,
    /// The person this bundle belongs to. Checked on import against the
    /// identity the importing node holds -- an import that would graft
    /// one person's data onto another's node is refused, not merged.
    pub subject_did: String,
    pub sections: BTreeMap<String, SectionDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub manifest: BundleManifest,
    /// Section name -> the section's documents, in the order the exporter
    /// wrote them. Order is part of the hashed bytes.
    pub sections: BTreeMap<String, Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("bundle version {0} is not understood by this build")]
    UnknownBundleVersion(u32),
    #[error("bundle json: {0}")]
    Json(String),
    #[error("section '{0}' has a digest but no content")]
    MissingSection(String),
    #[error("section '{0}' has content but no digest")]
    UndeclaredSection(String),
    #[error("section '{section}': {count} records, manifest says {expected}")]
    CountMismatch { section: String, count: u64, expected: u64 },
    #[error("section '{section}': content hash does not match the manifest")]
    DigestMismatch { section: String },
    #[error("bundle belongs to '{subject}', this node holds '{holder}'")]
    WrongSubject { subject: String, holder: String },
}

impl Bundle {
    /// One hash definition, shared with the envelope's own record id.
    pub fn digest(schema_version: u32, records: &[Value]) -> Result<SectionDigest, BundleError>;
    /// Every check that does not need a node: version, section symmetry,
    /// counts, hashes. `subject_did` is checked by the caller, which is
    /// the only party that knows who this node holds.
    pub fn check_integrity(&self) -> Result<(), BundleError>;
    pub fn to_json(&self) -> Result<String, BundleError>;
    pub fn from_json(s: &str) -> Result<Self, BundleError>;
}
```

`digest` is `content_digest(SECTION_DIGEST_PREFIX, &json!(records))`,
mapping the envelope crate's error. `check_integrity` pseudo-code:

```
if manifest.bundle_version != BUNDLE_VERSION -> UnknownBundleVersion
for (name, declared) in &manifest.sections:
    let records = sections.get(name) ?: return MissingSection(name)
    if records.len() as u64 != declared.record_count -> CountMismatch
    if Bundle::digest(declared.schema_version, records)?.digest != declared.digest
                                                    -> DigestMismatch
for name in sections.keys():
    if !manifest.sections.contains_key(name) -> UndeclaredSection(name)
Ok(())
```

`produced_at_secs` is deliberately **outside** every digest: it is the one
field that must vary between two runs, and hashing it would make an
otherwise reproducible bundle irreproducible.

### 3.7 `crates/roym_core/src/signing.rs` — new file, the certificate store (`D-C4-4`)

```rust
//! One record-signing certificate per service, held in that service's own
//! encrypted storage.
//!
//! The certificate carries no private key, and it certifies exactly one
//! key -- this service's own -- so it is useless anywhere else. The
//! signing host still takes it as a parameter on every call; this module
//! is only where the guest keeps that parameter between the ceremony that
//! mints it and the call that uses it.

pub const CERTIFICATES: &str = "signing_certificates";
/// One row, always. A second person on one installation is out of scope
/// for the first release, and a fixed id makes that visible rather than
/// letting a second row appear unnoticed.
pub const CERTIFICATE_ID: &str = "current";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCertificate {
    /// The `did:key` this certificate certifies -- this service's own
    /// signing key as it stood when the certificate was installed.
    pub signing_did: String,
    pub master_did: String,
    pub issued_at_secs: u64,
    pub expires_at_secs: u64,
    /// The certificate JSON exactly as minted, handed to the signing host
    /// unchanged.
    pub certificate: String,
}

/// `Stale` is a real state, not a theoretical one: the signing key is
/// re-derived from the recorded service owner on every call, so changing
/// a service's owner re-keys it and every stored certificate stops
/// matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum CertificateStatus {
    Missing,
    Stale { installed_for: String, current: String },
    Expired { expires_at_secs: u64 },
    Installed { master_did: String, expires_at_secs: u64, near_expiry: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    #[error("signing is not enrolled on this service")]
    NotEnrolled,
    #[error("the installed certificate expired at {0}")]
    Expired(u64),
    #[error("the installed certificate certifies '{installed_for}', but this service now signs with '{current}'")]
    Stale { installed_for: String, current: String },
    #[error("certificate rejected: {0}")]
    Rejected(String),
    #[error("this installation has no recorded owner, so it cannot sign as anyone")]
    NoOwner,
    #[error("storage: {0}")]
    Storage(String),
}

pub async fn install<H: AppHost>(host: &H, certificate_json: &str, now_secs: u64)
    -> Result<StoredCertificate, CertificateError>;
pub async fn status<H: AppHost>(host: &H, now_secs: u64)
    -> Result<CertificateStatus, CertificateError>;
/// The principal to sign the person's records with right now, plus the
/// master DID those records will be issued by.
pub async fn person_principal<H: AppHost>(host: &H, now_secs: u64)
    -> Result<(Principal, String), CertificateError>;
/// The DID this installation belongs to. Every "who is the person?"
/// question in the product goes through here, so there is one answer and
/// one place to change when more than one person is supported.
pub async fn owner_did<H: AppHost>(host: &H) -> Result<String, CertificateError>;
```

`install` pseudo-code — note the deliberately absent lower-bound check
(`D-C4-6`):

```
if certificate_json.len() > 16 * 1024 -> Rejected("too large")   // mirrors the
                                          // host's own delegation-size cap
let cert = DelegationCertificate::from_json(certificate_json)
               .map_err(|e| Rejected(e.to_string()))?
let id = host.signing_identity().await.map_err(...)?;
let owner = id.owner_did.ok_or(NoOwner)?;
if cert.temporary_did != id.signing_did
    -> Rejected("certifies <x>, not this service's signing key <y>")
if cert.master_did != owner
    -> Rejected("names master <m>, this installation's owner is <o>")
cert.verify_chain(&cert.master_did, &[SCOPE_RECORD_SIGNING])
    .map_err(|e| Rejected(e.to_string()))?
// Expiry only. "Not yet valid" is the host's to judge: it stamps records
// from a clock this component cannot read, so a lower-bound check here
// can refuse a certificate the signer would accept, for no safety gain.
if now_secs >= cert.expires_at_secs
    -> Rejected("already expired at <t>")
ensure_collection(host, CERTIFICATES).await?
host.put(CERTIFICATES, RecordWriteValue {
    id: CERTIFICATE_ID, payload: serde_json::to_vec(&StoredCertificate { … })? }).await
```

`status` pseudo-code:

```
ensure_collection(host, CERTIFICATES).await?
let Some(row) = host.get(CERTIFICATES, CERTIFICATE_ID).await? else { return Missing };
let stored: StoredCertificate = serde_json::from_slice(&row.payload)?;
let id = host.signing_identity().await?;
if stored.signing_did != id.signing_did
    { return Stale { installed_for: stored.signing_did, current: id.signing_did } }
if now_secs >= stored.expires_at_secs { return Expired { … } }
Installed { master_did: stored.master_did, expires_at_secs: stored.expires_at_secs,
            near_expiry: stored.expires_at_secs.saturating_sub(now_secs) < 6 * 3600 }
```

`person_principal` maps `Missing`→`NotEnrolled`, `Expired`→`Expired`,
`Stale`→`Stale`, and `Installed`→`(Principal::Delegated(stored.certificate),
master_did)`.

**The shared verb handler**, mounted by `profile` in C4 and by whichever
service needs it later:

```rust
/// The two certificate verbs, identical wherever they are mounted.
/// `prefix` is the mounting service's own method prefix, so the same code
/// answers under two names on two services. Returns `None` when
/// `req.method` is not one of them, so a caller tries this first and
/// falls through to its own table.
pub async fn handle_certificate_verb<H: AppHost>(
    host: &H,
    prefix: &str,
    req: &Request,
) -> Option<Response>;
```

Pseudo-code:

```
let suffix = req.method.strip_prefix(prefix)?;
match suffix {
    "signing-status" => {
        let id = host.signing_identity().await;         // -> internal_error on Err
        match status(host, clock::now_secs()).await {
            Ok(s)  => Some(Response::ok(json!({ "signing_did": …, "pubkey_hex": …,
                                                "owner_did": …, "certificate": s }))),
            Err(e) => Some(Response::internal_error(e.to_string())),
        }
    }
    "install-signing-certificate" => {
        let Some(cert) = req.params.get("certificate").and_then(|v| v.as_str())
            else { return Some(Response::invalid_params("certificate is required")) };
        match install(host, cert, clock::now_secs()).await {
            Ok(s)  => Some(Response::ok(json!({ "master_did": s.master_did,
                                                "expires_at_secs": s.expires_at_secs }))),
            Err(CertificateError::Rejected(m)) => Some(Response::invalid_params(m)),
            Err(e) => Some(Response::internal_error(e.to_string())),
        }
    }
    _ => None,
}
```

### 3.8 `crates/roym_core/src/router.rs` — prefixes and the auth column (`D-C4-2`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodAuth {
    /// Reachable with no session. Nothing here reads or writes anything
    /// belonging to a person.
    Public,
    /// Requires a verified person session whose subject is the DID this
    /// installation is recorded as belonging to.
    Owner,
}

const ROUTES: &[(&str, Service, MethodAuth)] = &[
    ("conversation.", CONVERSATION, MethodAuth::Owner),
    ("profile.",      PROFILE,      MethodAuth::Owner),
    ("contacts.",     PROFILE,      MethodAuth::Owner),
    ("block.",        PROFILE,      MethodAuth::Owner),
    ("report.",       PROFILE,      MethodAuth::Owner),
    ("listing.",      CATALOG,      MethodAuth::Owner),
    ("availability.", CATALOG,      MethodAuth::Owner),
    ("request.",      TRANSACTION,  MethodAuth::Owner),
    ("quote.",        TRANSACTION,  MethodAuth::Owner),
    ("agreement.",    TRANSACTION,  MethodAuth::Owner),
    ("receipt.",      TRANSACTION,  MethodAuth::Owner),
    ("directory.",    DIRECTORY,    MethodAuth::Owner),
];

/// Methods a person may reach before signing in. Full method names, never
/// prefixes: an exception granted to a prefix is an exception granted to
/// methods nobody has written yet.
const PUBLIC_METHODS: &[&str] = &["profile.policy"];

pub fn route(method: &str) -> Option<Service>;
pub fn method_auth(method: &str) -> Option<MethodAuth>;
```

> **`catalog.` and `transaction.` prefixes are not added.** An earlier
> draft added them so `<service>.install-signing-certificate` could reach
> every sibling. `D-C4-4` mounts the certificate verbs on `profile` only,
> so those prefixes would route to services with no method under them.
> C5 adds each prefix with its first real verb.

`method_auth` pseudo-code:

```
if PUBLIC_METHODS.contains(&method) { return Some(MethodAuth::Public) }
ROUTES.iter().find(|(p, _, _)| method.starts_with(p)).map(|(_, _, a)| *a)
```

New tests beside the existing five:
`every_public_method_is_routable`;
`every_route_prefix_has_an_auth_classification` (trivially true by the
tuple, asserted so a later `Option` cannot creep in);
`method_auth_is_none_for_an_unroutable_method`.
`manifest_depends_on_equals_siblings` is unchanged (`D-C4-11`).

### 3.9 `crates/roym_core/src/lib.rs`

```diff
 pub mod card;
+pub mod backup;
+pub mod clock;
 pub mod dual_build;
 pub mod envelope;
+pub mod person;
 pub mod record;
 pub mod router;
+pub mod safety;
 pub mod services;
+pub mod signing;
```

---

## §4 `syneroym-roym-profile` — the product surface

`crates/roym_profile/src/app.rs` is rewritten. `SCHEMA_VERSION` goes from
`1` to `2` — it exists to be bumped by whichever slice changes what the
service stores.

### 4.1 Collections

| Collection | Id | Payload | Indexes |
|---|---|---|---|
| `profiles` | the subject's master DID (the owner's, or a peer's) | `{ envelope: String, record_id: String, verified_at_secs: u64 }` | — |
| `profile_history` | `record_id` | `{ envelope: String }` — append-only, never deleted | — |
| `contacts` | the contact's master DID | `ContactRow` (§4.3) | `favourite` (boolean) |
| `blocks` | `did:<master-did>` or `addr:<address>` | `BlockRow` | `at_secs` (numeric) |
| `reports` | `report_id` | `ReportRow` | `status` (string), `at_secs` (numeric) |
| `contact_attempts` | `<sender_key>:<at_secs>:<n>` | `{ sender_key, at_secs }` | `sender_key` (string), `at_secs` (numeric) |
| `settings` | `contact_limits` | `ContactLimits` | — |
| `signing_certificates` | `current` | `StoredCertificate` | — |

All created with `ensure_collection` on first use (`F7`).

> **The two-copies cost, stated rather than discovered.** `profiles` holds
> the current envelope and `profile_history` holds every version including
> the current one. A profile envelope is bounded by the host's 64 KiB
> payload ceiling and in practice is under 2 KiB, and a person edits a
> profile a handful of times — a few tens of kilobytes for the life of an
> installation. A much smaller version of the honest cost `D-06C-5`
> requires C5 to state for message bodies.

### 4.2 Verb table

| Method | Auth | Effect |
|---|---|---|
| `profile.get` | Owner | `{ person_did? }` → the stored envelope. Omitted means the owner's own; a peer's comes from `contacts.upsert` (`D-C4-17`). |
| `profile.set` | Owner | `{ display_name, about?, conversation_address?, locale? }` → signs a new `profile` record, supersedes the previous, stores both rows. Refuses `signing-not-enrolled` (`D-C4-6`). |
| `profile.policy` | **Public** | This node's retention, deletion, block, and one-person statement. No storage read. |
| `profile.export` | Owner | The four sections as a `Bundle`. |
| `profile.import` | Owner | `{ bundle }` → integrity-checked restore. |
| `profile.signing-status` | Owner | §3.7's shared handler. |
| `profile.install-signing-certificate` | Owner | §3.7's shared handler. |
| `profile.ping` | Owner | Kept: `roym_app_e2e.rs` and the parity suite drive it. |
| `contacts.list` | Owner | `{ favourites_only? }` → `[ContactRow]`. |
| `contacts.get` | Owner | `{ person_did }`. |
| `contacts.upsert` | Owner | `{ person_did, display_name?, conversation_address?, favourite?, profile_envelope? }` — see §4.5. |
| `contacts.remove` | Owner | `{ person_did }`. |
| `contacts.resolve-address` | Owner | `{ person_did }` → the address to hand `open-direct`. Gap 5's read side. |
| `contacts.admit-first-contact` | Owner | `{ sender_person_did?, sender_address }` → `Admission`, recording the attempt when it allows. C5's inbox is the real caller. |
| `contacts.limits` | Owner | → the recipient's current `ContactLimits`. |
| `contacts.set-limits` | Owner | `{ window_secs, max_per_window }` → validated and stored (`D-C4-16`). |
| `block.add` | Owner | `{ person_did?, address?, reason? }` — at least one of the two. |
| `block.remove` | Owner | same key shape. |
| `block.list` | Owner | `[BlockRow]`. |
| `block.check` | Owner | `{ person_did?, address? }` → `{ blocked, since_secs? }`. |
| `report.create` | Owner | `{ subject_kind, subject_id, category, details? }` → `{ report_id, status }`. |
| `report.list` | Owner | `{ status? }` → `[ReportRow]`. |
| `report.get` | Owner | `{ report_id }`. |
| `report.withdraw` | Owner | `{ report_id }` → status `withdrawn`. |

`subject_kind` ∈ `person` | `listing` | `message`; `category` ∈
`impersonation` | `fraud` | `harassment` | `unsafe-service` |
`illegal-content` — the five the spec names, and no others.

### 4.3 Row types (in `roym_profile::app` — nothing else reads them)

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ContactRow {
    person_did: String,
    display_name: Option<String>,
    conversation_address: String,
    favourite: bool,
    added_at_secs: u64,
    /// The `record_id` of the verified `profile` record this contact's
    /// address came from. `None` means the person typed it in, and the UI
    /// says so rather than showing a mark it has not earned.
    from_profile_record: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct BlockRow { key: String, person_did: Option<String>, address: Option<String>,
                  reason: Option<String>, at_secs: u64 }

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ReportRow { report_id: String, subject_kind: String, subject_id: String,
                   category: String, details: Option<String>,
                   status: String,          // "recorded" | "withdrawn"
                   at_secs: u64 }
```

`report_id` = `content_digest("rep_", &json!({subject_kind, subject_id,
category, details}))` — **no timestamp** (`D-C4-14`). Filing the same
report twice updates `at_secs` on one row rather than creating a second,
and the id is comparable across builds.

### 4.4 `profile.set` pseudo-code — the one flow that signs

```
let now = clock::now_secs();
let owner = signing::owner_did(host).await?;
let (principal, master) = signing::person_principal(host, now).await?;
// person_principal already refused with a message naming the fix; map
// NotEnrolled / Expired / Stale to distinct codes so the Hub can offer
// the right next step rather than one generic banner.

let address = params.conversation_address
    .or(current_owner_profile_address(host, &owner).await?)   // keep it on edit
    .ok_or(invalid_params("conversation_address is required for the first profile"))?;

let payload = ProfilePayload { display_name, about, conversation_address: address, locale };
payload.validate()?;                                          // -> invalid_params

let supersedes = current_owner_profile_record_id(host, &owner).await?;

let envelope_json = host.sign_record(WitRecordDraft {
    version: 1,
    record_type: RECORD_PROFILE.to_string(),
    subject: owner.clone(),
    payload: serde_json::to_string(&payload)?,     // the WIT payload is a JSON *string*
    expires_at_secs: None,
    supersedes,
}, principal).await?;

// Parse, do not verify. Verifying here would compare the host's stamped
// `issued_at_secs` against this component's own clock, and those are
// different clocks -- a freshly signed record can read as issued in the
// future. `record_id` needs no clock, and the parity suite verifies the
// envelope with a clock it controls.
let envelope = Envelope::from_json(&envelope_json)
    .map_err(|e| internal_error(format!("the host returned an envelope this build cannot \
                                         parse: {e}")))?;
if envelope.issuer != owner {
    return internal_error("the host signed under an issuer this service did not ask for");
}
let record_id = envelope.record_id()?;

ensure_collection(host, PROFILE_HISTORY).await?;
ensure_collection(host, PROFILES).await?;
host.put(PROFILE_HISTORY, RecordWriteValue {
    id: record_id.clone(), payload: envelope_json.clone().into_bytes() }).await?;
host.put(PROFILES, RecordWriteValue {
    id: owner.clone(),
    payload: serde_json::to_vec(&json!({ "envelope": envelope_json,
                                         "record_id": record_id,
                                         "verified_at_secs": now }))?,
}).await?;

Response::ok(json!({ "record_id": record_id, "envelope": envelope_json }))
```

**Two `put`s, no transaction.** `batch_mutate` takes one collection, so a
crash between them leaves a history row with no pointer. History first,
pointer second, is the safe order, and the next `profile.set` writes a new
pair. A choice, recorded, not an accident.

### 4.5 `contacts.upsert` pseudo-code (`D-C4-17`)

```
let now = clock::now_secs();
let (display_name, address, from_record) = match &params.profile_envelope {
    Some(json) => {
        // The peer's own signature is the only thing that makes this
        // address attributable. Verify against the DID the person named,
        // so a swapped envelope cannot rename a contact.
        let v = verify_json(json, &VerifyOptions::new(now).expecting(&params.person_did))
            .map_err(|e| invalid_params(format!("profile did not verify: {e}")))?;
        if v.record_type != RECORD_PROFILE || v.version != 1 {
            return invalid_params("not a profile record this build understands");
        }
        let p: ProfilePayload = serde_json::from_value(v.payload.clone())
            .map_err(|e| invalid_params(format!("profile payload: {e}")))?;
        p.validate()?;
        ensure_collection(host, PROFILES).await?;
        host.put(PROFILES, RecordWriteValue {
            id: params.person_did.clone(),
            payload: serde_json::to_vec(&json!({ "envelope": json,
                                                 "record_id": v.record_id,
                                                 "verified_at_secs": now }))?,
        }).await?;
        (Some(p.display_name), p.conversation_address, Some(v.record_id))
    }
    None => (
        params.display_name.clone(),
        params.conversation_address.clone()
            .ok_or(invalid_params("conversation_address is required without a profile"))?,
        None,
    ),
};
ensure_collection(host, CONTACTS).await?;
host.put(CONTACTS, RecordWriteValue {
    id: params.person_did.clone(),
    payload: serde_json::to_vec(&ContactRow {
        person_did: params.person_did, display_name, conversation_address: address,
        favourite: params.favourite.unwrap_or(false),
        added_at_secs: existing_added_at.unwrap_or(now),
        from_profile_record: from_record })?,
}).await
```

`verify_json` uses `EmptyRevocations` by default, so `revocation_status`
comes back `Unknown` — correct for C4, where no revocation source exists,
and the Hub renders it as unknown rather than as a positive default. C9
supplies the real source.

### 4.6 `contacts.admit-first-contact` pseudo-code

```
let now = clock::now_secs();
let limits = load_contact_limits(host).await?;              // defaults if absent
let key = params.sender_person_did.map(|d| format!("did:{d}"))
             .unwrap_or_else(|| format!("addr:{}", params.sender_address));
ensure_collection(host, BLOCKS).await?;
ensure_collection(host, CONTACT_ATTEMPTS).await?;

// A block on either the DID or the address blocks the contact.
let blocked = host.get(BLOCKS, &key).await?.is_some()
    || (params.sender_person_did.is_some()
        && host.get(BLOCKS, &format!("addr:{}", params.sender_address)).await?.is_some());

let floor = now.saturating_sub(limits.window_secs);
let page = host.query(CONTACT_ATTEMPTS, QueryOptions {
    filter: Some(json!({ "sender_key": key, "at_secs": { "$gte": floor } }).to_string()),
    limit:  Some(limits.max_per_window + 1),
    cursor: None,
}).await?;
let attempts: Vec<u64> = page.records.iter().filter_map(at_secs_of).collect();

match safety::admit_first_contact(blocked, &attempts, &limits, now) {
    Admission::Allow => {
        host.put(CONTACT_ATTEMPTS, RecordWriteValue {
            id: format!("{key}:{now}:{}", attempts.len()),
            payload: serde_json::to_vec(&json!({ "sender_key": key, "at_secs": now }))?,
        }).await?;
        Response::ok(json!({ "admission": "allow" }))
    }
    Admission::Blocked => Response::ok(json!({ "admission": "blocked" })),
    Admission::RateLimited { retry_after_secs } =>
        Response::ok(json!({ "admission": "rate-limited",
                             "retry_after_secs": retry_after_secs })),
}
```

Two properties this shape depends on, both stated: `limit` is `max + 1`
so the count is exact at the boundary without paging, and a refusal is an
`ok` result carrying a reason — never an `Err` — because the refusal has
to be relayed to the sender, and only a structured answer can be.

### 4.7 `profile.export` / `profile.import`

`export`:

```
let owner = signing::owner_did(host).await?;
let now = clock::now_secs();
// profile_history is deliberately not exported: every version it holds is
// reproducible from the records themselves, and what must reproduce is
// identity and history, not every superseded draft.
let sections = BTreeMap::from([
    (SECTION_PROFILE,  collect(host, PROFILES).await?),
    (SECTION_CONTACTS, collect(host, CONTACTS).await?),
    (SECTION_BLOCKS,   collect(host, BLOCKS).await?),
    (SECTION_REPORTS,  collect(host, REPORTS).await?),
]);
let manifest = BundleManifest {
    bundle_version: BUNDLE_VERSION, produced_at_secs: now, subject_did: owner,
    sections: sections.iter()
        .map(|(k, v)| Ok((k.to_string(), Bundle::digest(SCHEMA_VERSION, v)?)))
        .collect::<Result<_, _>>()?,
};
Response::ok(serde_json::to_value(Bundle { manifest, sections })?)
```

`collect` pages with `QueryOptions { filter: None, limit: Some(500),
cursor }` until `next_cursor` is `None` — never stopping at a short page,
per the host interface's own note, and emitting
`{ "id": …, "payload": <parsed JSON> }` per row so the section is stable
JSON rather than base64 bytes.

`import`:

```
let bundle = Bundle::from_json(...)?;
bundle.check_integrity()?;                                  // -> invalid_params
let owner = signing::owner_did(host).await?;
if bundle.manifest.subject_did != owner {
    return invalid_params(WrongSubject { … }.to_string());  // never merge
}
for (name, records) in &bundle.sections {
    let collection = match name.as_str() {
        SECTION_PROFILE => PROFILES, SECTION_CONTACTS => CONTACTS,
        SECTION_BLOCKS  => BLOCKS,   SECTION_REPORTS  => REPORTS,
        other => return invalid_params(format!("unknown section '{other}'")),
    };
    ensure_collection(host, collection).await?;
    for chunk in records.chunks(100) {
        host.batch_mutate(collection.into(),
            chunk.iter().map(to_put_mutation).collect::<Result<Vec<_>, _>>()?).await?;
    }
}
// Re-verify every restored profile envelope. A restore that reproduces
// bytes without reproducing the verdict is not a restore.
let verified = reverify_profiles(host, clock::now_secs()).await?;
Response::ok(json!({ "sections": …, "profiles_verified": verified }))
```

**The unknown-section rule is a refusal, not a skip.** A bundle written by
a later slice importing into a C4 build must fail loudly: a silent skip is
data loss that looks like success.

### 4.8 The other four siblings

**Unchanged in C4.** `D-C4-4` mounts the certificate verbs on `profile`
alone, so `roym_catalog`, `roym_conversation`, `roym_transaction`, and
`roym_directory` are untouched by this slice.

---

## §5 `syneroym-roym-web` — the authorization gate

`crates/roym_web/src/app.rs`. Both `rpc` (the HTTP path) and `invoke` (the
proxied path) gain the same gate through one helper, so they cannot
diverge.

```rust
/// How this request is admitted, or the refusal to answer with. The
/// owner is resolved once per request from this service's own signing
/// identity -- one host call, not one per method.
enum Admitted { Yes, NoSession, NotOwner, NoOwnerRecorded }

async fn admit<H: AppHost>(
    host: &H,
    method: &str,
    caller: Option<&CallerIdentity>,
) -> Admitted {
    match router::method_auth(method) {
        None | Some(MethodAuth::Public) => Admitted::Yes,
        Some(MethodAuth::Owner) => {
            let Some(c) = caller else { return Admitted::NoSession };
            if c.auth != CallerAuth::Delegated { return Admitted::NoSession }
            match host.signing_identity().await.ok().and_then(|i| i.owner_did) {
                None => Admitted::NoOwnerRecorded,
                Some(owner) if owner == c.did => Admitted::Yes,
                Some(_) => Admitted::NotOwner,
            }
        }
    }
}
```

Codes, chosen so the Hub can act on them rather than show one banner:

| Outcome | Code | Message |
|---|---|---|
| `NoSession` | `-32010` | `not signed in` |
| `NotOwner` | `-32011` | `this installation belongs to another person` |
| `NoOwnerRecorded` | `-32012` | `this installation has no recorded owner` |

`NotOwner` deliberately does not name the owner: the message goes to
somebody who has already been told they are not it.

`session.whoami` is handled before routing and stays reachable with no
session — it is how the Hub discovers there is none. `GET /health` is
untouched.

**`invoke`'s path carries no caller at all** (`F4`), so a sibling calling
`web` gets `NoSession` for every `Owner` method. That is correct, and the
parity suite asserts it: `web`'s HTTP path and its `invoke` path are not
interchangeable once authorization exists, and saying so in a test stops
C5 from assuming they are.

---

## §6 `syneroym-identity` — the encrypted identity backup

### 6.1 `crates/identity/Cargo.toml`

```toml
[dependencies]
aes-gcm = { workspace = true, optional = true }
# ... existing, unchanged ...

[features]
default = []
# The person-facing encrypted key backup. Off by default so the
# `wasm32-wasip2` build of `syneroym-signed-record` -- which depends on
# this crate -- never links a cipher it cannot use: a component holds no
# private key, so it has nothing to back up.
backup = ["dep:aes-gcm"]
```

**Enabled by `apps/roymctl` and nothing else.** `crates/sdk` has no
consumer for it; an earlier draft said it did.

### 6.2 `crates/identity/src/backup.rs` — new file

```rust
#![cfg(feature = "backup")]
//! An encrypted, transportable copy of a person's master key.
//!
//! Encrypted under a randomly generated 32-byte **recovery key**, shown to
//! the person once and never stored. Not under a passphrase: this
//! workspace has no password KDF, and a passphrase would add a
//! parameter-versioning problem and a strength failure mode nothing here
//! can measure. A `kdf` value this build does not know is refused, never
//! guessed.

pub const IDENTITY_BACKUP_VERSION: u32 = 1;
pub const KDF_HKDF_SHA256: &str = "hkdf-sha256";
pub const CIPHER_AES_256_GCM: &str = "aes-256-gcm";
const HKDF_INFO: &[u8] = b"syneroym-identity-backup-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityBackup {
    pub backup_version: u32,
    /// The DID this backup restores. Public, and bound into the AEAD's
    /// additional data, so a backup cannot be relabelled as another
    /// person's without decryption failing.
    pub did: String,
    pub kdf: String,
    pub cipher: String,
    pub salt_z32: String,      // 16 bytes
    pub nonce_z32: String,     // 12 bytes
    pub ciphertext_z32: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup version {0} is not understood by this build")] UnknownVersion(u32),
    #[error("unknown kdf '{0}'")] UnknownKdf(String),
    #[error("unknown cipher '{0}'")] UnknownCipher(String),
    #[error("recovery key is not 32 bytes of z-base-32")] RecoveryKey,
    #[error("could not decrypt: wrong recovery key, or the backup was altered")] Decrypt,
    #[error("backup json: {0}")] Json(String),
    #[error("restored key does not produce the DID this backup names")] DidMismatch,
}

pub fn generate_recovery_key() -> Result<[u8; 32], BackupError>;
/// z-base-32, grouped `xxxxx-xxxxx-…` for reading aloud. Groups are
/// cosmetic: decoding strips `-` and whitespace and is case-insensitive,
/// because a person will retype this.
pub fn encode_recovery_key(key: &[u8; 32]) -> String;
pub fn decode_recovery_key(s: &str) -> Result<[u8; 32], BackupError>;
pub fn export(identity: &Identity, recovery_key: &[u8; 32]) -> Result<IdentityBackup, BackupError>;
pub fn import(backup: &IdentityBackup, recovery_key: &[u8; 32]) -> Result<Identity, BackupError>;
```

`export` pseudo-code:

```
let did = substrate::derive_did_key(&identity.public_key());
let mut salt = [0u8; 16];  getrandom::fill(&mut salt)?;
let mut nonce = [0u8; 12]; getrandom::fill(&mut nonce)?;
let mut key = [0u8; 32];
Hkdf::<Sha256>::new(Some(&salt), recovery_key).expand(HKDF_INFO, &mut key)?;
let aad = aad_bytes(IDENTITY_BACKUP_VERSION, &did, KDF_HKDF_SHA256,
                    CIPHER_AES_256_GCM, &z32(salt));
let mut secret = identity.to_bytes();
let ct = Aes256Gcm::new(&key).encrypt(&nonce, Payload { msg: &secret, aad: &aad })?;
secret.zeroize(); key.zeroize();
IdentityBackup { …, ciphertext_z32: z32(ct) }
```

`aad_bytes` is the canonical JSON of the header fields, through
`substrate::canonicalize_json_value` — one canonicalizer, again.

`import` reverses it, then **checks the restored key derives `backup.did`**
and returns `DidMismatch` otherwise. A backup that decrypts to the wrong
key must not silently install it.

Unit tests: round-trip; a flipped ciphertext byte fails `Decrypt`; a
changed `did` fails `Decrypt` (via the AAD, before the DID check);
`kdf: "argon2id"` fails `UnknownKdf` rather than being tried;
`backup_version: 2` fails `UnknownVersion`; encode/decode round-trip
through a lowercased, regrouped, whitespace-y retyping.

### 6.3 `crates/identity/src/lib.rs`

```diff
+#[cfg(feature = "backup")]
+pub mod backup;
```

---

## §7 `roymctl`

### 7.1 `apps/roymctl/src/commands/identity.rs` — two new subcommands

```rust
    /// Write an encrypted, transportable copy of a local identity. Prints
    /// a recovery key once; without it the backup cannot be opened, and
    /// nothing on this machine or any node can recover it.
    Export {
        #[arg(long)] name: String,
        #[arg(long, default_value = "identity-backup.json")] out: PathBuf,
    },
    /// Restore an identity from `identity export`'s output.
    Import {
        #[arg(long)] name: String,
        #[arg(long, value_name = "PATH")] r#in: PathBuf,
        /// The recovery key printed by `identity export`.
        #[arg(long)] recovery_key: String,
    },
```

`Export`: load `<dir>/identities/<name>.key`; generate a recovery key;
`backup::export`; write the JSON at mode 0600; print the recovery key with
a one-line warning that it is shown once. `Import`: read the JSON;
`backup::import`; **refuse if `<dir>/identities/<name>.key` already
exists** — never overwrite a key; `save_to_path`; print the restored DID.

`--recovery-key` is required rather than prompted: no prompt crate is in
the workspace, and adding one for this is a dependency for a convenience.
The help text says to pass it through a shell variable rather than
inline.

**Call sites:** the `handle` match gains two arms.
`apps/roymctl/Cargo.toml` changes `syneroym-identity` to
`{ workspace = true, features = ["backup"] }`.

### 7.2 `apps/roymctl/src/commands/roym.rs` — new file (`D-C4-5`)

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum RoymCommands {
    /// Mint and install a record-signing delegation on this
    /// installation's Roym services, so the substrate can sign records as
    /// you.
    ///
    /// Needs your master key file on this machine: only your master key
    /// can mint the certificate, and it must be the key that deployed
    /// Roym. A browser cannot do this -- its session key is a delegate,
    /// and a delegate cannot re-delegate.
    EnrolSigning {
        /// Name of the local identity that deployed this installation.
        #[arg(long)] r#as: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)] gateway_url: String,
        /// The Hub's gateway host. `roymctl alias <web-service-did>`
        /// prints it.
        #[arg(long)] host: String,
        #[arg(long, default_value_t = 720)] expires_hours: u64,
    },
    /// Report the certificate state of this installation's Roym services.
    SigningStatus { /* gateway_url, host */ },
}
```

`EnrolSigning` pseudo-code:

```
let master = Identity::load_from_path(dir.join("identities").join(format!("{as}.key")))?;
let master_did = substrate::derive_did_key(&master.public_key());
let token = read_session_token()?;              // what `session login` wrote
// One prefix today. C5/C6 add theirs when they mount the verbs.
for prefix in ENROLLABLE_PREFIXES /* ["profile"] */ {
    let s = rpc(&gateway_url, &host, &token, &format!("{prefix}.signing-status"),
                json!({})).await?;
    if s["owner_did"] != master_did {
        bail!("service '{prefix}' is owned by {}, not {master_did}. Roym must be \
               deployed by the person who uses it -- a certificate for this pair \
               would be refused at sign time.", s["owner_did"]);
    }
    let key = substrate::resolve_did_key(s["signing_did"].as_str().context(...)?)?;
    let cert = DelegationCertificate::issue(
        &master, key, expires_hours * 3600, SCOPE_RECORD_SIGNING.to_string())?;
    rpc(&gateway_url, &host, &token, &format!("{prefix}.install-signing-certificate"),
        json!({ "certificate": cert.to_json()? })).await?;
    println!("{prefix}: enrolled until {}", cert.expires_at_secs);
}
```

The default is 720 hours (30 days), not `certify-signing`'s 24: a person
will not re-run a ceremony daily, and a record signed under a certificate
that has since expired still verifies (C3 §3.4 step 5 and its regression
test). Re-running the command is the renewal path.

**Call sites:** `apps/roymctl/src/commands.rs` — `pub mod roym;` beside
the other nine, and a `Roym { command: roym::RoymCommands }` arm in
`Commands` plus its dispatch in the `handle` match at `:240`'s level.

---

## §8 `syneroym-control-plane` — the external-caller gate (`D-C4-9`)

`crates/control_plane/src/synsvc_native.rs`.

```rust
/// A native-capability method that acts with the service's own key, or
/// hands back its own secret, is reachable only by the service itself or
/// by the person who owns it. `signing/identity` is deliberately not on
/// this list: it returns public identifiers, and gating a read of public
/// data buys nothing.
///
/// `system:<service_id>` is what a call originating inside this service's
/// own guest carries -- either forwarded unchanged by a self-proxy, or
/// synthesized by the sandbox when no caller reached the guest at all.
/// Every other value is a caller from outside.
fn admit_privileged_capability(&self, caller: &CallerContext) -> RpcResult<()> {
    if caller.caller_did == format!("system:{}", self.service_id) {
        return Ok(());
    }
    let owner = self
        .record_signer
        .get()
        .and_then(|s| s.identity(&self.service_id).ok())
        .and_then(|i| i.owner_did);
    match owner {
        Some(o) if o == caller.caller_did => Ok(()),
        _ => Err(RpcError::Custom(
            PERMISSION_DENIED_CODE,
            format!(
                "'{}' may not reach this capability on service '{}': it is neither the \
                 service itself nor its recorded owner",
                caller.caller_did, self.service_id
            ),
            None,
        )),
    }
}
```

Applied at exactly two points:

```diff
     async fn dispatch_signing(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
         let Some(signer) = self.record_signer.get().cloned() else { … };
         match invocation.method.as_str() {
             "sign-record" => {
+                self.admit_privileged_capability(&invocation.caller)?;
                 let params = …
```

```diff
     async fn dispatch_vault(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
         match invocation.method.as_str() {
             "reveal" => {
+                self.admit_privileged_capability(&invocation.caller)?;
                 #[derive(serde::Deserialize)] struct Req { key: String }
```

`vault` has no owner source independent of the record signer, so a node
with no record signer configured refuses `vault/reveal` for every
non-self caller. That is the correct direction, and it matches
`dispatch_signing`'s existing behaviour when no signer is configured.

**Call sites and tests that must change:**
- `crates/substrate/tests/record_signing_e2e.rs` step 8 — the assertion
  inverts from "succeeds" to "refused, naming the owner rule". Step 9's
  *outcome* is unchanged but its *reason* moves: `stranger` is now refused
  by the owner gate before the caller-binding check runs, so the message
  assertion must change. Both are the point of the slice.
- `F11`: nothing else in the tree reaches either method externally.
- Two new unit tests beside `dispatch_signing`'s existing ones: the
  `system:<id>` caller is admitted; an arbitrary verified DID is refused
  with `PERMISSION_DENIED_CODE`.

**This gate has no native-build counterpart, and that is not a permitted
difference.** A natively linked Roym service has no `SynSvcNativeService`
at all — `init_roym` registers it in `native_dispatch` under its own `api`
interface, and its `AppSigning` reaches `HostState` directly. So there is
nothing on the native side that this gate would have had to close.

---

## §9 Small corrections in existing files

Two planning-identifier slips (`F13`).

The first is inside the file §3.2 already rewrites, so it is fixed there
rather than twice. Repeated here only because it is the one edit whose
*point* is removing a planning reference, and a diff that replaced it
with a pointer to a plan section would put another one straight back:

```diff
-/// Known Roym record types (spec D-C3-14).
+/// Every record type this product produces, and the version each is
+/// produced at today. Fixed, like the card table: a record of an unlisted
+/// type, or a listed type at an unlisted version, is not understood
+/// rather than guessed.
+///
+/// `profile` proves who published a person's card and the conversation
+/// address they claim. It is how a stranger reached by direct link gets
+/// an address they can attribute, with no directory involved.
```

The second is in a file nothing else in this slice touches:

```diff
-# In Slice C2, routes on `web` are public: true so local gateway requests
-# (and unauthenticated endpoints like health) reach web without pre-gating.
-# Sibling services are internal/private. Person session authentication is
-# verified in application logic and fine-grained session auth gating is
-# target for C4 (tracked in deferred backlog).
+# `/rpc` is public at the stream boundary so an unauthenticated request
+# can reach `web` at all -- the login screen and the policy statement are
+# served before any session exists. `web` itself refuses every method
+# that touches a person's data unless the request carries a verified
+# session whose subject is this installation's recorded owner; the
+# routing table names which methods those are. Sibling services stay
+# internal/private and answer only what reaches them through `web` or,
+# for the public ones, over the wire.
```

No `depends_on` edges are added (`D-C4-11`), so `init_roym` is unchanged.

---

## §10 The Hub

`crates/roym_web/ui/src/`. Plain TypeScript, no framework, `textContent`
only — the rule that governs card values governs every value here.

| File | Contents |
|---|---|
| `src/rpc.ts` | **New.** One `call(method, params)` over `POST /rpc` with `authHeaders()`; maps `-32010`/`-32011`/`-32012` onto typed `NotSignedIn` / `NotOwner` / `NoOwner`. **`main.ts`'s existing inline `fetch("/rpc", …)` in `renderHome` moves here** — that is an edit to an existing call site, not only a new file. |
| `src/screens/setup.ts` | Shown when `profile.signing-status` is not `installed`. Names the command (`roymctl roym enrol-signing --as <name> --host <hub host>`), states that the browser cannot do this because it holds no master key, and offers "check again". |
| `src/screens/profile.ts` | `profile.get` / `profile.set`. Shows the record id and the issuer. |
| `src/screens/contacts.ts` | `contacts.*`, favourites, the resolved address, and a "paste a profile" input that feeds `contacts.upsert`'s `profile_envelope`. |
| `src/screens/safety.ts` | `block.*`, `report.*`, `contacts.limits`/`set-limits`, and `profile.policy`'s text. The block copy uses `D-06C-8`'s wording verbatim. |
| `src/screens/backup.ts` | Export/import of the **app-data** bundle only, as a file download and upload. It never touches the identity key, and says so. |

`src/main.ts` becomes a three-state shell: not signed in → the existing
login screen; signed in but not enrolled → `setup`; signed in and enrolled
→ a tab bar over profile / contacts / safety / backup, with the card
gallery behind a "components" tab so the existing Playwright card cases
keep passing unchanged.

The existing vitest suite (`login.test.ts`, `link.test.ts`) must keep
passing; `rpc.ts` gets its own `rpc.test.ts` covering the three code
mappings.

---

## §11 Tests

### 11.1 What each suite is for

| Suite | Proves |
|---|---|
| `crates/roym_core/src/safety.rs` unit tests | Every branch of both limiters and of `ContactLimits::validate`, at and around the boundary, with no host. |
| `crates/roym_core/src/backup.rs` unit tests | Every `BundleError` variant; a digest that changes when any record changes; a section present in one half and not the other; `produced_at_secs` not affecting any digest. |
| `crates/roym_core/src/signing.rs` unit tests | Every `CertificateStatus` and `CertificateError` against a `#[cfg(test)]` `AppHost` stub — the first in this crate; a module, not a new dev-dependency. |
| `crates/roym_core/src/router.rs` unit tests | The auth table is total over `ROUTES`; every public method routes. |
| `crates/signed_record` unit tests | `content_digest` is stable, prefix-separated, and `record_id`'s existing tests still pass after the refactor. |
| `crates/identity/src/backup.rs` unit tests | Round-trip, tamper, wrong key, unknown kdf/cipher/version, DID mismatch, recovery-key retyping. |
| `crates/control_plane/src/synsvc_native.rs` unit tests | `admit_privileged_capability` admits self and owner, refuses anyone else, on both interfaces. |
| `crates/roym_web/tests/dual_build_parity.rs` | §11.2. |
| `crates/substrate/tests/roym_identity_e2e.rs` (new) | §11.3 — the real ceremony against a real substrate and gateway. |
| `crates/substrate/tests/roym_app_e2e.rs` | Unchanged assertions, changed harness (§11.3). |
| `crates/substrate/tests/record_signing_e2e.rs` | §8's inverted steps 8 and 9. |
| `crates/roym_web/ui` vitest | `rpc.ts`'s error mapping, plus the existing two files. |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | §11.4. |

### 11.2 New parity scenarios

Appended to `crates/roym_web/tests/dual_build_parity.rs`. The harness must
gain three things it does not have:

1. `endpoint_registry.set_owner(...)` for all six service ids, to one test
   person DID, on both stacks.
2. A `NodeRecordSigner` on both stacks from the same `Arc<Identity>` with
   `RecordClock::Fixed(F)` — copy the setup from
   `crates/app_host_native/tests/dual_build_parity.rs`.
3. **One certificate, minted once, installed byte-identically on both
   stacks.** Its window must straddle both the real wall clock (which the
   guest reads at install) and `F` (which the host reads at sign) — C3
   §13.2's own recipe: `expires_hours = ceil((F - now) / 3600) + 24`.

> **What is and is not compared** (`D-C4-12`). The signed envelope's
> timestamp is the host's, so it is pinned and compared byte for byte.
> Every other artifact carries a guest timestamp that differs between the
> two runs, so the suite compares it through
> `strip_volatile(&mut Value)` — which removes `at_secs`,
> `added_at_secs`, `verified_at_secs`, `produced_at_secs`, and
> `retry_after_secs` — and asserts those fields separately for presence
> and plausibility. This is a property of the two clocks, not of the two
> builds; a shared, injectable guest clock would mean shipping a
> production config key whose only purpose is to lie about the time.

| # | Scenario | Assertion |
|---|---|---|
| 11 | `profile.set` with no certificate installed | Same `signing-not-enrolled` refusal and code on both. |
| 12 | Install the certificate, then `profile.set` | The two envelopes are **byte-identical**, and both verify with `expected_issuer` = the owner under `VerifyOptions::new(F)`. |
| 13 | `profile.set` twice, second with a changed `display_name` | Same `supersedes` chain and same two `record_id`s on both; the first envelope still in `profile_history`. |
| 14 | `profile.get` after 13 | Same envelope and `record_id` on both (`verified_at_secs` stripped). |
| 15 | `install-signing-certificate` with a certificate over another key | Same `invalid_params` message on both. |
| 16 | …with a `service-instance`-scoped certificate | Same refusal on both. |
| 17 | …naming a master that is not the recorded owner | Same refusal on both. |
| 18 | …already past its `expires_at_secs` | Same refusal on both; the "not yet valid" case is deliberately **not** a scenario (`D-C4-6`). |
| 19 | `contacts.upsert` with no envelope, then `contacts.resolve-address` | Same address on both, `from_profile_record` `null`. |
| 20 | `contacts.upsert` with scenario 12's envelope for that person | Same address on both, taken from the record, and `from_profile_record` equals scenario 12's `record_id`. |
| 21 | `contacts.upsert` with an envelope whose issuer is a different DID than `person_did` | Same `invalid_params` on both. |
| 22 | `contacts.set-limits { window_secs: 3600, max_per_window: 2 }` then four `admit-first-contact` calls | `allow, allow, rate-limited, rate-limited` on both, and `retry_after_secs` on both is in `(3600-10, 3600]`. |
| 23 | `contacts.set-limits` with a window below the floor | Same `invalid_params` on both. |
| 24 | `block.add` then `admit-first-contact` for that sender | `blocked` on both, and no attempt row is written. |
| 25 | `block.add` by address, `admit-first-contact` by DID + that address | `blocked` on both. |
| 26 | `report.create` twice with identical content | Same `report_id` on both builds **and** between the two calls; one row. |
| 27 | `report.withdraw` then `report.list` | Same status on both. |
| 28 | `profile.export` after 12, 19, 24, 26 | The two bundles are identical after `strip_volatile`, and `check_integrity` passes on both. |
| 29 | `profile.import` of scenario 28's bundle into a second, empty stack of the same build | Same section counts, and the profile re-verifies. |
| 30 | `profile.import` of a bundle with one record edited and the manifest untouched | Same `DigestMismatch` on both. |
| 31 | `profile.import` of a bundle whose `subject_did` is another person | Same `WrongSubject` refusal on both. |
| 32 | `POST /rpc` `profile.get` with no caller | Same `-32010` on both. |
| 33 | `POST /rpc` `profile.get` with a caller who is not the owner | Same `-32011` on both. |
| 34 | `POST /rpc` `profile.policy` with no caller | Same 200 and same body on both. |
| 35 | `web::invoke` (the proxied path) for `profile.get` | Same `-32010` on both — the two paths are not interchangeable (§5). |

### 11.3 Substrate e2e

**`crates/substrate/tests/roym_app_e2e.rs` — harness change, no assertion
change.** `F14`: the deploy runs as `ctx.owner` while the person logging
in is a generated `alice`, so after `D-C4-1`/`D-C4-2` every `Owner`
method under alice's cookie would answer `-32011`, and the existing
`profile.ping` step would fail. `AuthService::login_local` reads the key
file on each login, so the fix is ordering, not restructuring:

```diff
-    let alice = Identity::generate().unwrap();
-    let alice_did = substrate::derive_did_key(&alice.public_key());
-    let alice_key_path = ids_dir.join("alice.key");
-    alice.save_to_path(&alice_key_path).unwrap();
-
     let ctx = SubstrateTestContext::setup_with(iroh_port, reg_port, gw_port, move |cfg| { … }).await;
+
+    // The person and the deployer must be the same DID: the substrate
+    // derives a service's signing key under its recorded owner, and only
+    // that owner's master key can mint a certificate the signer will
+    // accept. `login_local` reads this file per login, so writing it
+    // after setup is enough.
+    let alice = Identity::from_bytes(&ctx.owner.to_bytes());
+    let alice_did = ctx.owner_did.clone();
+    alice.save_to_path(ids_dir.join("alice.key")).unwrap();
```

Every later `alice_did` assertion holds unchanged.

**`crates/substrate/tests/roym_identity_e2e.rs` — new.** Same harness,
one substrate, WASM build, one gateway.

```
 1. Deploy Roym as the substrate owner, and use that identity as the
    person -> owner_of == the person's DID on every service.
 2. `session login`; keep the token.
 3. `profile.signing-status`
      -> "missing", owner_did == the person, signing_did resolves.
 4. `profile.set` before enrolment              -> "signing-not-enrolled".
 5. Run what `roym enrol-signing` performs      -> "installed".
 6. `profile.set { display_name, conversation_address }`
      -> the envelope verifies with expected_issuer = the person, and its
         payload carries the address that was sent.
 7. A second identity, `stranger`, logs in at the same auth service and
    calls `profile.get`                          -> -32011, against two
         real sessions rather than a hand-built CallerIdentity.
 8. `stranger` mints a certificate over `profile`'s published signing key
    (public, from step 3) naming itself as master, and calls
    `profile.install-signing-certificate`
      -> refused: not this installation's owner. The certificate store
         cannot be seeded by anyone but the person.
 9. `stranger` calls `signing/sign-record` on the profile service
    directly over native dispatch, as-principal = service
      -> permission denied. C3's posture, inverted.
10. `profile.export`; wipe the app's storage; `profile.import`
      -> the profile record re-verifies with the same record_id.
11. `identity export` the person's key, `identity import` it into a fresh
    directory, and repeat step 5 from the restored key alone
      -> succeeds, and the certificate it mints is accepted. This is
         "restore on a clean node reproduces identity".
12. Using the substrate's own node identity, and without the person's
    master key, attempt to mint a certificate for `profile`
      -> refused at install (wrong master) and, if the row is written
         directly, refused at sign time by the certificate's own chain
         check. "No operator can impersonate", proven twice.
```

### 11.4 Browser cases in `roym-hub.spec.ts`

- Signed in with no certificate → the setup screen names the enrolment
  command and offers no way to mint one in the browser.
- After enrolment (driven by the harness through the API), the profile
  screen saves and reloads a profile.
- A contact whose address the person typed shows as unverified; one added
  by pasting a verified profile envelope does not.
- The safety screen's block copy contains `D-06C-8`'s sentence and does
  **not** contain any claim that the sender was prevented from sending —
  asserted as a string, so the wording cannot drift silently.
- A profile whose `display_name` contains `<img onerror=…>` renders as
  text: no element created, no request made. Same shape as the existing
  card-safety case.

### 11.5 Failure-and-security-matrix rows C4 closes

| Row | How |
|---|---|
| **11** (a blocked sender) | Partially: the list, the decision, and the wording. The inbox half is C5 — §18 **C**. |
| **12** (flooding) | The contact-rate half fully, including a recipient-settable ceiling and a refusal returned to the caller. The publication half ships as the rule with no caller. |
| **13** (import reproduces verification status) | For the C4 sections (scenario 29, e2e step 10). |
| **17** (restart mid-session) | The Hub's three-state shell treats "not signed in" as ordinary. |
| **19** (build divergence) | 25 new parity scenarios. |

---

## §12 Order of work

Each step compiles and its own tests pass before the next.

1. `syneroym-signed-record`: `content_digest`, the `record_id` refactor,
   the three re-exports, and the `wasm32-wasip2` build check (§3.1).
2. `syneroym-identity`: the `backup` feature, `backup.rs`, unit tests
   (§6). Independent of everything else.
3. `roym_core`: `clock`, `person`, `safety`, `backup`, the corrected
   `record` (§3.2–§3.6) and their unit tests. No host involved.
4. `roym_core::signing` and its `#[cfg(test)]` `AppHost` stub (§3.7).
5. `roym_core::router`'s auth column (§3.8). The workspace stops
   compiling here until step 7 — `ROUTES`' tuple arity changed.
6. `roym_profile`: the whole surface (§4.1–§4.7).
7. `roym_web`'s gate (§5). Workspace compiles again.
8. The two comment corrections (§9).
9. `roymctl`: `identity export`/`import`, then `roym enrol-signing` (§7).
10. `synsvc_native`'s gate, its unit tests, and the two inverted
    `record_signing_e2e.rs` steps (§8).
11. **`roym_app_e2e.rs`'s harness change** (§11.3). Do this immediately
    after step 7 if the suite is being run continuously — it is the one
    existing test `D-C4-2` breaks.
12. The parity harness's owner / signer / certificate setup, then the
    scenarios (§11.2).
13. `roym_identity_e2e.rs` (§11.3).
14. The Hub (§10), its vitest additions, and `roym-hub.spec.ts` (§11.4).
    Rebuild with `mise run build:roym-ui` and `mise run build:roym`; run
    `mise run test:roym-ui`.
15. `cargo xtask check-roym-deps`, then the full gate: `cargo +nightly fmt
    --all`, `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace`, `cargo audit`, `cargo deny check licenses`,
    `mise run test:e2e`.
16. Docs and backlog (§14).

Steps 1–2, 3–4, and 10 are independent. Step 5 is the choke point: it
breaks the build until step 7.

---

## §13 Permitted differences (WASM vs native)

To be appended to `status.md`'s §14 list.

1. **Nothing new from the app's own code.** Both builds run the identical
   `roym_core` and `roym_profile` sources over the same `AppHost` traits.
2. **One inherited difference now has teeth.** A natively linked Roym has
   no deploy record, so `owner_of` is `None` unless `[roles.roym]
   owner_did` is set — and after C4 that is not a subtle difference in a
   derived key, it is the difference between a working installation and
   one where every `Owner` method answers `-32012`. The native build must
   set it; the parity harness sets it explicitly on both stacks.
3. **§8's gate has no native counterpart, and that is not a divergence.**
   A natively linked service has no `SynSvcNativeService`, so there was
   never an external native-dispatch path into its `signing`/`vault` to
   close. Both builds' guest path reaches `HostState` directly and is
   unaffected.
4. **The two builds' guest clocks are not synchronized**, which is a
   property of wall clocks rather than of the shim. §11.2's preamble says
   what the suite compares as a result.

---

## §14 Documents and backlog owed

| Document | Edit |
|---|---|
| [status.md](status.md) | A C4 section: what shipped, §11.5's matrix rows, §13's permitted differences, `F13`'s correction to what C3's section recorded about `roym_core::record`, and — explicitly — what C4 did **not** close: the inbox half of row 11, and the publication half of row 12. |
| [task.md](task.md) | Gap 5 recorded as closed on the *product* side, naming `roym_core::person` and the `profile` record, and the open design point "which service owns the person→conversation-address mapping" answered. **The "Owed as slices land" table's C4 row must be amended**: it says `[PRD-SAF]` moves to "Recently resolved" at C4, and §14's disposition below splits it instead. Leaving the two documents disagreeing is worse than editing the table. |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | **Records table gains a `profile` row**: signed by the person; proves who published this card and the Conversation address they claim; does not prove the person is who they say they are outside this network. **Profile & Contacts' API column gains `report.*`.** The Safety section gains one sentence that the first release binds one installation to one person, who must also be the party that deployed it. |
| [deferred-backlog.md](../../deferred-backlog.md) | §3's three C4-targeted rows: the **certificate-lifecycle** row moves to "Recently resolved" with what shipped (per-service app storage, a CLI ceremony, `signing-status`/`install-signing-certificate`) **and what did not** — a browser-only person still cannot enrol, which becomes its own row, trigger *"a person is expected to run Roym without a shell on the substrate host"*. The **external-caller gate** row moves to "Recently resolved", covering `signing/sign-record` and `vault/reveal`, with `signing/identity` deliberately still open and why. The **internal-caller binding** row is **restated, not resolved**: the reachability is gone, the check is not, and it stays targeted at M6's cross-service caller-identity pass. §7's `/rpc` gating row moves to "Recently resolved", with two residuals named: `/rpc` stays `public: true` at the stream boundary (§18 **E**), and the gate lives in `web`, so it says nothing about the wire ingress into `catalog`/`conversation`/`directory` — that becomes a new row targeted at **C5**. §10's `[PRD-SAF]` row is **partially** resolved: block, report, contact limits, and policy disclosure ship; the inbox enforcement point retargets to **C5** and the publication limiter to **C5/C6**. **New rows:** (a) a service can resolve a dependency it never declared (`F2`), trigger *"a binding check enforces declared dependencies"*; (b) the app-data bundle carries no signature until C8; (c) a passphrase-wrapped identity backup is not built, trigger *"a person cannot be trusted to keep a recovery key"*; (d) `contact_attempts` is never pruned — the window query is bounded but the table grows; (e) **`roymctl identity certify-signing` and `sdk::certify_record_signing` are now a second, divergent ceremony** — they mint against `resolve-instance-identity`, which `F6` says can disagree with the signer, while `roym enrol-signing` asks the signer itself. Trigger *"a second app needs record signing"*, at which point one of the two should win; until then the developer guide points only at the Roym path. (f) the managed-guild path (a SynOrg hosting a provider's substrate) cannot work while the person must be the deployer (`D-C4-1`), trigger *"a hosted provider is a supported deployment"*. |
| [developer-guide.md](../../../developer-guide.md) | The enrolment ceremony beside the existing session-login documentation: `identity create` → `identity export` → deploy Roym **as that identity** → `session delegate` → `session login` → `roym enrol-signing`. State that `identity certify-signing` is the general primitive and not the Roym path. |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | The architecture paragraph says the five non-`web` services "answer `<name>.ping` only". After C4 that is wrong for `profile`. One sentence. |

**No new ADR.** C4 adds one record type, one storage location, and one
admission rule, each decided inside this plan the way `D-06C-3` decided
the card set. Nothing here is a wire format a party outside Roym consumes.

---

## §15 What "done" means for C4

1. A person creates an identity, deploys Roym as it, exports an encrypted
   backup, restores it in a clean directory, and enrols signing from the
   restored key alone.
2. `profile.set` produces a `profile` record that verifies with
   `expected_issuer` = the person, and refuses outright when no usable
   certificate is installed.
3. Contacts carry a conversation address, and the Hub distinguishes one
   that came from a verified `profile` record from one the person typed —
   with a code path that can produce both.
4. Block, report, and a recipient-settable contact rate limit work, the
   refusal is returned to the caller rather than swallowed, and the
   product's own words match `D-06C-8`, asserted as a string.
5. Every `Owner` method refuses an unauthenticated and a non-owner
   caller, on both builds and through both of `web`'s paths.
6. `signing/sign-record` and `vault/reveal` refuse a caller who is
   neither the service nor its owner; `signing/identity` still answers.
7. A same-version export/import round-trip reproduces the C4 sections
   **and their verification status**, and refuses a bundle belonging to
   another person or with an altered record.
8. All 25 new parity scenarios pass identically on both builds, including
   the byte-identity assertion on the signed envelope.
9. `cargo xtask check-roym-deps` is clean, and a grep over
   `crates/roym_*`, `crates/roym_core/app/`, and every file this slice
   touched finds **no planning identifier in any name *or comment*** —
   `M0[0-9]`, `\bR[1-4]\b`, `\bC[0-9]`, `D-C[0-9]`, `D-0[0-9]`, `Slice `.
   ADR references are the only permitted exception. `F13`'s two existing
   slips are fixed by this slice.
10. The full gate in §12 step 15 is clean.
11. §14's documents and backlog rows are written — including the six new
    rows and the three things C4 explicitly did not close.

---

## §16 What C4 deliberately does not build

- **Roym's own copy of conversation content** and the inbox that consults
  the block list. C5.
- **Listing publication**, and therefore any caller for
  `safety::admit_publication`. C5/C6.
- **The certificate verbs on `catalog`/`conversation`/`transaction`/
  `directory`.** The handler is shared and ready; mounting it on a
  `public` service before wire-side authorization exists would add a
  wire-reachable write verb (`F3`). C5/C6 mount it with that
  authorization.
- **A SynOrg's policy disclosure** — rules, retention, dispute path,
  support contact, shown before a provider joins. C6. `profile.policy`
  discloses *this node's* policy only.
- **Report delivery to a SynOrg**, and the statuses a SynOrg produces.
  C6 and C9.
- **Multi-person installations**, and the managed-guild deployment the
  spec's encryption table contemplates. `D-C4-1` states the limit.
- **A browser path to enrolment.** A delegate cannot re-delegate.
- **Any cross-node authorization** on `catalog`/`conversation`/
  `directory`'s wire-facing `api.invoke`. `F3` names it; C5 owns it.

---

## §17 Open questions for the executor

Choices, not defects. Each has a recommended answer and the plan works
either way.

1. **`expires_hours` default for `roym enrol-signing`.** The plan says
   720. Shorter is defensible if renewal is scripted; longer is not,
   because a stolen certificate is usable for its whole window against a
   key that does not rotate.
2. **Whether `conversation.` should be `Owner`.** It is, because nothing
   under it exists yet. By `F3` a foreign caller does not traverse `web`,
   so the answer is probably still `Owner` when C5 adds methods — but C5
   decides with a real method in hand.
3. **Whether `profile_history` should be exported.** §4.7 says no.
   Reversing it costs one section and makes the bundle grow without bound
   over a long-lived installation.

---

## §18 Ambiguities and staleness in the input documents

Flagged rather than guessed. **A**, **B**, and **J** change what gets
built.

**A. R1 row 1's acceptance test says "identity **and history**", and
history does not exist until C5.** `D-06C-5` puts Roym's own copy of
conversation content in C5, and the host's conversation store cannot be
written back into (Gap 3). So the "history" half is **not meetable in C4
by construction**, whatever C4 does. This plan builds the bundle so C5
adds sections rather than reshaping it, proves restore for the sections
C4 owns, and leaves R1 row 1's gate closing in C5. Stated rather than
quietly failed, following `D-06C-2`'s precedent. The row needs no
rewording — it is an ordering fact, not an unmeetable test — but
`status.md` must say which slice closes it.

**B. `D-C3-12`'s claim that "C4 is the first slice that signs a person
record through a sibling call" is true only because this plan makes it
true.** None of the spec's nine record types is produced by C4: `listing`
is C5, the transaction family C7/C8, the SynOrg family C9. Read strictly,
C4 signs nothing and C5 becomes the first signer — which would leave C4
shipping the certificate lifecycle C3 declared its *hard prerequisite*
with no flow exercising it. `D-C4-3` resolves this by adding the `profile`
record, which Gap 5 independently wants and `D-C4-17` gives a writer.
A reviewer who refuses the tenth type must move the certificate lifecycle
to C5; "ship it unexercised in C4" is not a third position.

**C. R1 row 6's acceptance test cannot close in C4, for the same reason
as A.** *"A blocked sender's messages never reach the recipient's inbox"*
needs an inbox, and `D-06C-8` puts enforcement at Roym's own inbox, which
is `D-06C-5`'s and therefore C5's. C4 ships the list, the decision
function, the wording, and `contacts.admit-first-contact` for C5 to call.

**D. `roym_core::record` shipped with a different signature than C3's plan
specified, and C3's `status.md` records the plan's version.** `F13`. C4
corrects the tree to the planned shape, because the `(type, version)` pair
is what the unknown-*version* rule needs and what `card::CARD_TYPES`
already does.

**E. `roym.toml`'s comment and the backlog row both promise more than C4
delivers.** The row says C4 introduces *"product-level person session
authorization checks, fine-grained capability gating across all `/rpc`
methods, and verifies stream caller invariants"*. `D-C4-2` delivers the
first two. The third — refusing an inbound stream with `caller == None`
at the stream capability boundary — is **not** delivered: `/rpc` stays
`public: true`, because making it non-public would refuse the Hub's first
load and `profile.policy` before any session exists. Resolved with that
residual named.

**F. `task.md`'s open design point about where the app-owned message copy
lives is C5's, not C4's** — but the *mechanism* (a bundle with
per-section digests, sections added by later slices) is fixed here, so C5
inherits a format rather than choosing one.

**G. `task.md`'s Migration-impact section still reads as if C1 was the
last time `AppHost`'s supertrait list grew.** C3 §18 **B** flagged it
once. C4 adds no trait, so the list stops at nine — noted so the next
slice does not re-flag a section that is now correct.

**H. Two backlog rows about the signer's key derivation are load-bearing
for C4 and neither is targeted at it.** *"Dynamic record signer key
re-derivation on service owner change"* (C7/C8) and *"One-way persistence
of `roym.owner_did`"* (TBD) both mean the signing key can change under a
stored certificate. `D-C4-4`'s `CertificateStatus::Stale` is C4's answer —
the state is detected and reported rather than producing records nobody
can verify. Named so C7/C8 knows a consumer already exists.

**I. C2's `status.md` says a sibling never learns who is asking, and C4
does not change that.** `D-C4-1` establishes *who this installation
belongs to*, which is a different question, answerable from host-attested
state. It does not tell `profile` that this particular call came from that
person — only `web` knows that, and only for its own HTTP ingress.

**J. `D-C3-13`'s "whoever can certify is the DID that deployed the
service" is stated as a constraint on the CLI; it is a constraint on the
product.** `F14` shows it reaching all the way to the person's login: the
person must be the deployer, or nothing they do can be signed as them. The
existing `roym_app_e2e.rs` violates it, which is how it was found. The
spec's own encryption table contemplates a SynOrg hosting a provider's
substrate — a deployment this makes impossible, since the host would be
the deployer and therefore the person. `D-C4-1` states the limit and §14
gives it a backlog row. This deserved a decision row in `task.md` and did
not get one.

---

## §19 Review comments not adopted, and why

**`roymctl alias` does exist.** A review pass flagged §7.2's reference to
`roymctl alias <web-service-did>` as naming a verb that is not in the
tree. It is: `Commands::Alias` is declared at
`apps/roymctl/src/commands.rs:64` and handled at `:240`, printing
`util::generate_service_host(nickname, service_id, interface, domain)` —
the unscoped gateway host form C2's `F8` says the Hub is reached at. No
new verb is needed and the reference stands.

**The four `depends_on` edges were dropped rather than kept.** The same
pass accepted them as an honesty fix for `F2`. On reflection they are a
declaration nothing traverses, which the plan refuses everywhere else, so
`D-C4-11` records the gap in the backlog instead and lets C5 declare the
edge it actually uses. This is a change of position from the first draft,
recorded so a reader of both is not confused about which is current.
