# M06C Slice C3 — Signed Records: The Host Signing Interface and the Record Envelope: Implementation Plan

> **Scope.** [task.md](task.md)'s **C3** row: a host interface that signs
> under a principal the substrate holds the key for and never hands key
> material out (`D-06C-4`, Gap 1); one canonical record envelope with a
> stable byte encoding and an explicit `version` field (`D-06C-1`); and
> guest-side verification of signature, issuer, scope, expiry, and
> revocation. Gates: **C1** (the dual-build shim) and **C1.1**
> ([ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md),
> which settles the identity model this interface is specified against).
>
> **No product records.** C3 defines the envelope and proves the interface
> against `test-components/dual-build-fixture`, exactly as C1 proved its
> four interfaces. `listing`, `quote`, `agreement-receipt` and the rest are
> C4–C9's. The only Roym-side change here is a dependency and a re-export,
> so C4 starts from a compiling base.
>
> **Read §18 first if you are executing this plan.** Two claims in the
> input documents do not hold against the tree, and one of them changes the
> shape of the interface.

---

## §0 What C1 / C1.1 / C2 handed C3, and what is missing

| Handed over | Where |
|---|---|
| A trait-per-host-interface shim with two implementors, `GuestHost` and `NativeAppHost` | `crates/app_host/src/lib.rs:37` (`AppHost`), `crates/app_host/src/guest.rs`, `crates/app_host_native/src/host.rs` |
| One host-capability implementation both builds share: the native shim delegates every call to `sandbox_wasm`'s `HostState` | `crates/app_host_native/src/host.rs:528` (`AppVault` → `HostVault::reveal`) |
| A worked example of a WIT package deliberately **outside** the `host-environment` world, with separate guest and host bindgen modules | `syneroym:conversation` — `crates/wit_interfaces/src/conversation.rs`, `crates/wit_interfaces/src/conversation_host.rs`, linked in `crates/sandbox_wasm/src/engine.rs:761` |
| A per-service derived signing key, deterministic and redeploy-stable | `Identity::derive_service_identity` (`crates/identity/src/keys.rs:240`), used by `SynSvcNativeService::new` (`crates/control_plane/src/synsvc_native.rs:540`) |
| A worked "sign under a delegated key, ship the certificate inside the signed payload" record | `RelationshipProof::sign` / `::verify` (`crates/rpc/src/relationship_proof.rs:100`, `:130`) |
| The identity model records are specified against | ADR-0024 §3 + Amendment 2: the caller is a verified `syneroym_session` token resolved by the router into `CallerIdentity { did: person_did, auth: Delegated }` |
| A JSON canonicalization + `did:key` + ed25519 verification vocabulary | `syneroym_identity::substrate::{canonicalize_json_value, verify_json_signature, derive_did_key, resolve_did_key}` |

Missing, and C3's to build:

1. **No signing verb anywhere a guest can reach.** `syneroym:vault`'s
   whole surface is `reveal` (`crates/wit_interfaces/wit/host/deps/vault/vault.wit`),
   and the native verb table is `data-layer`, `vault`, `app-config`,
   `blob-store`, `messaging`, `conversation`
   (`crates/control_plane/src/synsvc_native.rs:1616-1626`).
2. **No record envelope.** Nothing in the tree defines a signed,
   versioned, append-only record. `RelationshipProof` is the nearest
   shape and is a 60-second internal authorization artifact, not a
   durable product record.
3. **No guest-reachable verification.** `syneroym-identity` and
   `syneroym-ucan` are host crates; D3 keeps Roym out of both.
4. **No delegation from a person to a service's signing key.** The only
   certificate the substrate holds is ADR-0020's instance certificate,
   whose `master_did` is required to equal the `service_id`
   (`crates/control_plane/src/service/orchestration.rs:2989`) — it is a
   *member master*, never a person. See **F4**.

---

## §1 Findings from reading the tree

Verified 2026-08-31 against `main` at `edc76de`.

### F1 — the substrate already holds exactly one signing key per service, and it is the right one

`SynSvcNativeService::new` derives
`node_identity.derive_service_identity(owner_did, &service_id)` once at
construction (`crates/control_plane/src/synsvc_native.rs:540`), where
`owner_did` is `EndpointRegistry::owner_of(service_id)` — the DID that
deployed the service. `ControlPlaneService::instance_identity`
(`crates/control_plane/src/service/orchestration.rs:1193`) derives the same
key for a *caller*, and `resolve-instance-identity` publishes its `did:key`
and hex pubkey so a master holder can certify it without the substrate ever
seeing the master. The key is deterministic, redeploy-stable for the same
owner, and never persisted. C3 needs no new key material and no new key
storage.

### F2 — `RelationshipProof` is the working precedent for the whole envelope

`sign(instance, certificate, …)` sets `asserter_did` to the certificate's
master when one is given and to the signer's own DID otherwise, embeds the
certificate JSON **inside** the signed payload so it cannot be swapped, and
signs `Identity::sign_json` over the struct with `signature` set to `""`
(`crates/rpc/src/relationship_proof.rs:100`). `verify` resolves the signing
key through the certificate, checks the certificate's scope and window with
`DelegationCertificate::verify`, then checks the signature
(`:130`). C3's envelope is this shape, generalized and made durable. Do not
invent a second signing convention.

### F3 — `canonicalize_json_value` is **not** RFC 8785, whatever its doc comment says

`crates/identity/src/substrate.rs:180` sorts object keys recursively and
then hands the value to `serde_json::to_string`. It does not apply RFC
8785's number canonicalization (ES6 `Number::toString`) and does not
normalize string escapes. For integers and strings `serde_json`'s output is
deterministic and matches JCS; for a non-integer `f64` it is
`serde_json`-version-specific, and a producer or verifier in another
language will not reproduce it. **Consequence for C3:** the host must
refuse to sign a payload containing a non-integer number. That is cheap
now, impossible later, and it is the single most valuable entry in
`D-06C-4`'s "what does the host refuse to sign" list. (Prices become
integer minor units, which is what C8's transaction vertical wants anyway.)

### F4 — the ADR-0020 instance certificate's master is the **service**, not the person

`verify_installed_instance_cert` requires `cert.master_did == service_id`
(`crates/control_plane/src/service/orchestration.rs:2989`), and
`syneroym_sdk::deploy::certificate_over_instance_identity` refuses to mint
one otherwise (`crates/sdk/src/deploy.rs:655`). `service_id` for an
app member is the *minted member master* (`roymctl app deploy --mint-masters`),
so the installed certificate proves "this node's derived key speaks for
member master M", never "…for person P".

`EndpointRegistry::owner_of(service_id)` does record the deploying person's
DID — but it is a local bookkeeping string with no signature behind it. A
third party cannot verify it, so it cannot be an issuer.

**So "sign under the person's delegated key" has no existing artifact
behind it.** C3 must introduce the person→service-key delegation itself.
This is the finding that shapes the interface — see `D-C3-3`.

### F5 — a delegation certificate is a public artifact, so the *guest* can hold it

`DelegationCertificate` is `{master_did, temporary_did, issued_at_secs,
expires_at_secs, scope, signature}` (`crates/identity/src/delegation.rs:56`).
It contains no private key. A guest presenting one cannot forge an issuer,
because the host refuses any certificate whose `temporary_did` is not the
`did:key` of the exact key it is about to sign with — and only the named
master's private key can mint such a certificate. This removes the need for
a new registry slot, a new storage table, a new orchestrator verb, and a
new replay path. See `D-C3-4`.

### F6 — `HostState` is the one implementation both builds already go through

`crates/sandbox_wasm/src/host_capabilities.rs:233`. The WASM engine builds
one per invocation (`engine.rs:1286`); `NativeHostFactory::build_host_state`
builds the same struct with `max_memory_bytes: None`
(`crates/app_host_native/src/factory.rs:240`). Adding one field plus one
`with_…` setter — the `with_conversation` pattern at
`host_capabilities.rs:352` — gives both builds signing from a single
implementation, with no `HostState::new` signature change (it has ~20 call
sites, most of them tests).

### F7 — neither `AppSandboxEngine::init` nor `NativeHostFactory::new` can grow a parameter cheaply

`AppSandboxEngine::init` has **77** call sites; `NativeHostFactory::new`
has **12**. Both already use post-construction wiring for exactly this
reason (`self_weak`, `websocket_senders`, `service_proxy`,
`conversation`). C3 uses the same: `OnceLock` on the engine, `OnceLock` +
`set_record_signer` on the factory.

### F8 — `NATIVE_CAPABILITY_INTERFACES` is a fixed-size array, and every deployed service is registered under all of it

`crates/core/src/local_registry.rs:42` — `[&str; 7]`. Every deployed
service gets one `NativeHostChannel` endpoint per entry at deploy
(`crates/control_plane/src/service/orchestration.rs:2123`), and the same
list is filtered back out of `list`
(`orchestration.rs:3269`) and of "this service's one app-declared
interface" resolution (`local_registry.rs:212`). Adding `"signing"` means
the array literal's length changes to 8 and
`crates/router/tests/native_dispatch_identity.rs:246` iterates one more
entry. Nothing else reads the length.

### F9 — the guest native-capability gate needs nothing new

`ProxyRouter::check_native_capability_gate`
(`crates/router/src/proxy.rs:589`) refuses a proxy call to *another*
service's native capability and allows the same-service self-call, keyed
off `NATIVE_CAPABILITY_INTERFACES`. Adding `"signing"` to the array is the
whole change: a guest reaching another service's `signing` is refused with
`permission-denied` for free, and its own is reachable through its host
imports.

### F10 — `syneroym-identity` compiles for `wasm32-wasip2` today

`cargo build -p syneroym-identity --target wasm32-wasip2` succeeds (three
`unused` warnings on that target, all inside the `libc::mlock` block in
`crates/identity/src/keys.rs:8,24`). So a wasm-safe verification crate can
depend on it and reuse `DelegationCertificate::verify` and
`verify_json_signature` rather than re-implementing two security-critical
checks. Fix the three warnings in the same pass (`#[cfg]` the `io` import
and underscore-prefix `lock_memory`'s parameters on non-unix), because
`mise run build:roym` will start surfacing them.

### F11 — `xtask check-roym-deps` allowlists dependencies by exact name and does not look at dev-dependencies

`xtask/src/main.rs:70` — `allowed_target_independent` is
`["syneroym-app-host", "syneroym-roym-core", "serde", "serde_json",
"async-trait", "thiserror"]`. It reads `[dependencies]`, the two
`[target.…]` tables, and `[package.metadata.component.target.dependencies]`
— **never `[dev-dependencies]`**. So C3 must add its new crate to
`allowed_target_independent`, and a test-only `syneroym-identity`
dev-dependency in `roym_core` passes the check as written (deliberate: a
dev-dependency does not ship into the component, and generating a keypair
is the only way to test verification).

### F12 — the fixture is where a host interface is proven, and the parity suite drives it below the router

`crates/app_host_native/tests/dual_build_parity.rs` (34 cases) drives
`test-components/dual-build-fixture`'s `run(request)` verb table on both
builds and compares. `RevealSecret`/`ReadConfig` (fixture `app.rs:610-624`)
are the model for a new `SignRecord`/`VerifyRecord` pair.

### F13 — ed25519 signatures in this tree are deterministic, so two builds can produce byte-identical envelopes

`ed25519-dalek`'s `Signer::sign` is RFC 8032 deterministic. Given the same
node identity, the same `owner_did`, the same `service_id` and the same
`issued_at_secs`, the WASM and native builds produce the **same bytes**.
That makes a byte-identity parity assertion possible, but only with an
injected clock — hence `RecordClock` in `D-C3-8`.

### F14 — nothing gates `vault::reveal` on a capability, and `signing` must match that or diverge visibly

`vault::Host::reveal` (`host_capabilities.rs:501`) checks nothing but the
service's own DB. `signing` follows the same posture in C3 (only the
`read_only` stage-4 denial), because inventing a capability gate for one
interface while its sibling has none creates an inconsistency nobody
tested. Recorded as a backlog row, targeted at C4, which is where "may this
service sign as this person" becomes a real question.

### F15 — a native-capability interface is reachable by an authenticated *external* caller, not only by the guest

`crates/router/tests/native_dispatch_identity.rs:237` proves an anonymous
caller is refused for every entry of `NATIVE_CAPABILITY_INTERFACES`, and
`:256` proves an authenticated one is admitted. `check_native_capability_gate`
(`crates/router/src/proxy.rs:589`) constrains *guest* proxy calls; it does
not constrain a client the router already admitted to `svc/<service_id>`.
So adding `"signing"` to the array means any caller the handshake admits to
that service can ask it to sign — the same posture `vault/reveal` has
today.

C3 accepts that rather than inventing an exception (`D-C3-15`), and names
it: it is not a new class of exposure, it is the existing one applied to a
new verb, and the fix is the same capability gate `D-C3-12` defers to C4.
It is also what lets exit criterion 2's "a second client drives the same
flow" reach signing at all, and what makes §13.3's end-to-end test possible
without a WASM component.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C3-1** | **A new WIT package `syneroym:signing@0.1.0`, one interface, two functions: `sign-record` and `identity`. It is *not* added to the `host-environment` world.** | `task.md`'s Migration-impact note requires exactly this, and `syneroym:conversation` is the worked precedent: a component that does not import it deploys exactly as before. |
| **D-C3-2** | **There is no "sign these bytes" verb.** The guest hands the host a `record-draft`; the host builds the envelope, stamps `issuer` and `issued-at-secs`, attaches the delegation, signs, and returns the finished envelope JSON. A guest can never state its own issuer or its own timestamps. | `D-06C-4` asks C3 to decide "what it signs under, and what it refuses to sign". A blind byte-signing oracle answers neither: it cannot refuse an envelope it cannot parse, and it lets any service assert any issuer. |
| **D-C3-3** | **Two principals: `service` and `delegated`, not `service`/`person`/`member`.** `service` signs with the derived instance key, uncertified — the issuer is the instance `did:key`. `delegated` signs with the same key and carries a `DelegationCertificate` inside the envelope — the issuer is whatever master DID that certificate names. | Every one of the spec's nine record types is signed by a person or an organisation acting as one (`listing`→Provider, `membership-credential`/`revocation`/`moderation-decision`→SynOrg, `request`/`quote`/`agreement-receipt`/`payment-acknowledgement`/`fulfilment-receipt`→the two parties). A first pass named the second principal `person` and excluded a third, `member` (the ADR-0020 member master), on the reasoning that no record type is signed by one. **That reasoning does not survive `membership-credential`/`revocation`/`moderation-decision`**: a SynOrg needs a stable issuer DID that survives an administrator change, and neither of the two remaining candidates gives it one — the administrator's personal DID makes every credential personally attributable and breaks continuity on handover, and the Directory's own derived key changes if a different owner redeploys it (`derive_service_identity` keys on owner *and* `service_id`). The certificate format does not care whether the master DID it names belongs to a natural person or to a SynOrg's own dedicated minted master (an identity a SynOrg's administrator holds on the organisation's behalf, distinct from their personal DID, the same shape a member master already is) — nothing about `verify`'s checks depends on that distinction. So rather than adding a `member` variant that is untested until C6/C9 (the `D-C1-10` discipline this plan otherwise follows), the principal is named for what it structurally is — **a delegation from any master DID this service was handed a certificate for** — and C6/C9 use exactly the same `delegated` case for a SynOrg's own dedicated master. This is additive if it later turns out insufficient: `principal` is a WIT `variant`, so a genuinely new case is a backward-compatible addition, never a migration of anything already signed. `service` stays because `moderation-decision` is arguably the Directory *as a service* and because a node with no delegation must still be able to sign something and say so honestly. **Contradiction between D-C3-3 and D-C3-4, and its resolution:** A SynOrg with a dedicated master DID has `cert.master_did != admin_did`, so a session-authenticated administrator calling the directory service directly would fail D-C3-4's `cert.master_did == caller_did` check. The only path that works today is the `Internal` one — i.e. the administrator drives the action through the web service, which proxies to directory as `CallerBinding::Internal`, where the binding check does not apply. **Chosen position:** a SynOrg's org-master certificate is valid only on the `Internal` path; the administrator flow for org-signed records must always go through the web service (or another sibling), never as a direct authenticated call against the directory service. This is a constraint on the C6/C9 UI design, not on the signing interface. An \"acts-for\" delegation model (where an admin calling directly could be recognized as authorized to sign under the org master) is not built and is tracked as a future extension if direct-path SynOrg admin calls prove necessary. |
| **D-C3-4** | **The delegation is a *parameter*, not substrate state, and the host checks it against the calling session where one exists.** `principal::delegated(cert-json)`. The host verifies, per call, that the certificate is well-formed, in its validity window at the moment it signs, scoped `record-signing`, and certifies **the exact key it is about to sign with** — otherwise `no-delegation`. **When the caller arrived as a verified, externally-checked identity** (a session token or UCAN chain — `AuthLevel::Delegated`/`Ucan`, not a substrate-injected context), the host additionally requires `cert.master_did == caller_did` — otherwise `no-delegation`, naming the mismatch. A substrate-injected caller (a cross-service sibling call via `service_system`, a lifecycle hook, a stage-4 after-step) has no externally-verified DID to check against and is not held to this — see `D-C3-12` for what that residual still leaves open. **Coverage note:** this check fires only when a call reaches the signing host with a verified identity — i.e. when a session-authenticated caller (a browser, an external client) addresses the service directly. A guest's cross-service proxy call carries `CallerContext::service_system` (`host_capabilities.rs:1351-1355`), which maps to `AuthLevel::System`, which maps to `CallerBinding::Internal`. So catalog, profile, transaction, and directory — every service that will produce person-signed records in C4–C8 — always arrive at the signer as `Internal` and the binding check never runs. The check closes the direct-access case (e.g. web service, or an external client); D-C3-12 tracks what the sibling path still leaves open. **Comparison with D-04-02-h (`synsvc_native.rs:579-589`):** That rule forbids an `AuthLevel::System` carve-out in data-layer policy enforcement where synthesized `System` auth could become *more* permissive than a direct call. For signing, the `Internal` path skips the binding check (so it is more permissive than `Verified`), but it cannot be exploited as a bypass for two reasons: (1) a guest cannot launder its own session by self-proxying, because a self-proxy call forwards the *real* caller (`host_capabilities.rs:1351`), so it remains `Verified` and checked; and (2) a guest cannot reach another service's signing capability through the proxy at all, because `check_native_capability_gate` refuses all cross-service native-capability proxy calls (`proxy.rs:641-660`). The only consequence is that code executing inside a sibling service, once legitimately invoked, signs unchecked — which is exactly the `D-C3-12(b)` residual targeted at C4. | F5: the certificate carries no private key, so a guest holding it is safe on its own, but holding it is not the same as being the person it names — a certificate is bearer-shaped, and anyone who obtains the JSON can present it. The caller-binding check closes the case of a live, session-verified browser request presenting someone else's certificate, at the cost of one string comparison, using identity information (`AuthLevel`, `caller_did`) `HostState`/`CallerContext` already carry per invocation — no new mechanism. It avoids a new registry slot, a new `EndpointStorage` method triple, a new SQLite table, a new orchestrator verb, and a new boot-time replay path — none of which any acceptance test needs. It also does not preclude a multi-person node later, which a single per-service registry slot would. |
| **D-C3-5** | **A new scope, `record-signing`, and it is never a transport scope.** `SCOPE_RECORD_SIGNING` joins `crates/identity/src/delegation.rs`; `TRANSPORT_SCOPES` is **not** changed. | A certificate minted to sign records must not be replayable onto a connection preamble or accepted as an ADR-0020 instance certificate. `DelegationCertificate::verify` already enforces scope against a caller-supplied accepted set, so the separation costs one constant and one negative test. Reusing `service-instance` would overload an artifact the routing and authorization paths already read. |
| **D-C3-6** | **The envelope, its canonical bytes, and its verifier live in one new crate, `crates/signed_record` → `syneroym-signed-record`, which compiles for the host **and** for `wasm32-wasip2`.** It depends on `syneroym-identity` for `DelegationCertificate::verify`, `verify_json_signature`, `canonicalize_json_value` and `derive_did_key`. It exposes **no** function that takes an `Identity`. | One definition of the canonical bytes, or the host signs one thing and the guest verifies another. F10 proves the dependency compiles. Excluding any `Identity`-taking entry point is what keeps D3 true by construction: Roym links this crate and still has no way to sign — signing needs `Identity::sign`, which lives on the far side of the WIT boundary. |
| **D-C3-7** | **Verification is guest-side Rust, not a host interface.** `roym_core` re-exports `syneroym_signed_record`; both builds run the identical code. | `task.md`'s Gap 1 says so in as many words: "Checking an ed25519 signature needs only a public key, so a guest can vendor the crypto and do it itself, which is exactly what the spec's 'the consumer's own node verifies' rule requires." A host verify verb would also make the consumer's verdict depend on the node it is talking to, which is what failure-matrix row 2 forbids. |
| **D-C3-8** | **The host-side signer is a concrete `NodeRecordSigner` in `syneroym-core`, held `Arc` (not `Weak`, not `dyn`), with an injectable `RecordClock`.** Fields: `Arc<Identity>` (node identity) and `EndpointRegistry`. | It holds no reference back to the engine or the factory, so there is no cycle to guard against and the `Weak<dyn …>` dance `ConversationHost`/`ServiceProxy` need does not apply. `syneroym-core` already depends on `syneroym-identity` and owns `EndpointRegistry`, and both `HostState` and `NativeHostFactory` already depend on `syneroym-core`. `RecordClock::Fixed` is what makes F13's byte-identity parity assertion possible. `CallerBinding` lives here rather than reusing `syneroym_rpc::AuthLevel` because depending on `syneroym-rpc` would drag `syneroym-wit-interfaces` and `wasmtime` into `syneroym-core` — a transitive-weight problem, not a cycle (neither depends on the other today). |
| **D-C3-9** | **`record-id` is derived, never stored, and it is a hash of the *whole signed envelope*, signature included — there is no `record_id` field on `Envelope` to exclude.** `Envelope::record_id()` = `sha256` over the canonical bytes of the full envelope, z32-encoded, prefixed `rec_`. `supersedes` stores another record's derived id. **Named consequence, not a defect:** ed25519 signing in this tree is deterministic (F13), so two envelopes built from byte-identical drafts, signed by the same key under the same delegation at the same `issued_at_secs`, are byte-identical and share a `record_id` — a legitimate retry naturally collapses onto the same record rather than minting a second one. If a later slice needs two records with identical business content to be independently addressable (e.g. "the same quote sent twice as two separate offers"), the *producer* varies the payload or `subject` to make them distinct — the envelope does not manufacture uniqueness on its own, deliberately. C7/C8 must treat this as given when they design `supersedes` and any `record_id`-keyed storage, not discover it. | A stored id that can disagree with the content is a bug waiting for a slice that forgets to recompute it. Deriving it makes "this record is that record" a property of the bytes, and content-derived identity is exactly what makes a retried sign idempotent for free. |
| **D-C3-10** | **Two version fields, both explicit: `envelope_version` (the shape of the envelope, `1` today) and `version` (the record body's own schema version, producer-chosen, never `0`).** Neither has a serde default; an absent one fails to parse. | `D-06C-1`'s field is `version` — the record body's. The envelope's own shape needs its own, or the first envelope change silently reinterprets every stored record. Neither is a compatibility ladder: an unknown value is refused, never migrated (`UnknownEnvelopeVersion`), which is `D-06C-1`'s own rule applied to itself. Failure-matrix row 14 falls out of "no serde default". |
| **D-C3-11** | **The host refuses to sign a payload containing a non-integer number, a payload that is not a JSON object, a payload over 64 KiB or nested deeper than 32, a `record-type` outside `[a-z0-9-]{1,64}`, a `version` of `0`, a `subject` over 256 bytes, and an `expires-at-secs` already in the past.** | F3 makes the number rule load-bearing for reproducible canonical bytes. The rest is the parse-floor `D-06C-4` asks for, stated as a list a test can walk rather than as prose. |
| **D-C3-12** | **No capability gate on `signing` in C3 beyond the `read_only` (stage-4) denial and the `D-C3-4` caller-binding check.** Backlog row, targeted at **C4**. | F14/F15. `D-C3-4`'s check closes the case of a session-verified caller presenting a certificate naming a *different* master — the exposure `D-06C-4` is actually worried about. It does **not** close: (a) `principal::service` records — any caller the router admits to a service can make it sign under its own identity, unbounded, the same posture `vault/reveal` already has, honestly *not* the same stakes (`vault` leaks a secret the service already owns; `signing` produces a record attributed to that service's own DID, which is lower-severity than a forged person-issued record but still not "no one but this service can trigger this"); (b) a `delegated` record signed by an *internal* caller (no externally-verified DID to check) — this is **not** a C8 concern: C4 is the first slice that signs a person record through a sibling call (catalog, profile, transaction, or directory calling the signing host), and every such call arrives as `CallerBinding::Internal`. The residual is load-bearing from the moment C4 starts. C4 must decide whether an internal caller may drive `delegated` signing at all, or must be handed a verified identity some other way. Target **C4**, not a later slice. |
| **D-C3-13** | **`roymctl identity certify-signing` mints the delegation; no substrate-side install verb is added.** It queries `orchestrator/resolve-instance-identity` **as the identity issuing the delegation**, signs a `record-signing`-scoped certificate over the returned key, and prints the JSON. | The certificate is a parameter (`D-C3-4`), so nothing needs installing. Reusing `resolve-instance-identity` is what makes the minted certificate match the key the host will actually sign with. **Constraint, stated rather than discovered:** that verb derives under the *caller's* DID, and the signer derives under `owner_of(service_id)` — so whoever can certify is the DID that deployed the service. True for R1's one-person node; named in `signing-identity.owner-did` so an app can say whose signature it needs. **What this does not solve, named rather than waved at "C4":** Roym has six services, so one person needs up to six certificates — one per service, each with its own expiry, minted through a CLI that needs the person's master key file on the local machine. There is no storage for these certificates today (D-C3-4 keeps them out of substrate state on purpose), no renewal path, and no route for a browser-only Hub person — who never has a master key file to run this CLI with — to obtain one at all. C4 cannot ship a single record-producing flow without an answer to this, so it is a **named, hard prerequisite of C4**, not an incidental detail — see the matching row in §16. |
| **D-C3-14** | **C3 ships no Roym record types and no Roym verbs.** `roym_core` gains `src/record.rs` — the re-export plus the spec's nine record-type names as a constant — and the dependency. The interface is proven in `test-components/dual-build-fixture`. | The C1 shape: a host interface is proven against the fixture, not against a half-built product. `task.md` gives C3 no release, exactly like C1 and C2. |
| **D-C3-15** | **`signing` joins `NATIVE_CAPABILITY_INTERFACES` and gets a native-dispatch arm, with the same admission posture `vault` has.** A guest reaches its own through host imports or its own same-service self-proxy; another service's is refused; a client the router admitted to that service can reach it. | F15, and consistency: a reserved capability name with no dispatch arm would make a guest's own same-service self-proxy — which `check_native_capability_gate` explicitly permits (`crates/router/src/proxy.rs:3273`) — fail with `unknown interface`. Refusing external callers for `signing` alone while `vault/reveal` stays open would be an asymmetry with no rule behind it. The real fix is one capability gate across both, which is `D-C3-12`'s backlog row. |
| **D-C3-16** | **`RevocationSource` answers `Good` / `Revoked` / `Unknown` per check, never a bare `bool`, and `EmptyRevocations` answers `Unknown` for everything.** A `Revoked` verdict is a hard `VerifyError`; `Good` and `Unknown` both produce a `VerifiedRecord`, carrying which one it got as `revocation_status`. C9 supplies the real source; this slice ships the shape and both non-error branches. | `task.md`'s own rule — "missing evidence renders as unknown, never as a positive default" (C6's search-results row, D-06C-6's own §2 wording) — is stated at the UI layer everywhere it appears in the input docs, but the envelope is where "missing" first becomes representable at all. A `bool`-returning source cannot say "I don't know" — a caller who forgets to wire a real source today would silently get "not revoked" on every record, which is the opposite of the rule and is exactly the failure mode `EmptyRevocations` under the old signature could not be told apart from "checked, and clean". |

---

## §3 The new crate — `syneroym-signed-record`

### 3.1 `crates/signed_record/Cargo.toml`

```toml
[package]
name = "syneroym-signed-record"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
syneroym-identity.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
sha2.workspace = true
z32.workspace = true
thiserror.workspace = true

[dev-dependencies]
serde_json.workspace = true
```

Root `Cargo.toml`, `[workspace.dependencies]`, alphabetically among the
internal crates:

```toml
syneroym-signed-record = { path = "crates/signed_record" }
```

No `anyhow`: every error here is a typed enum a caller matches on.
No `ed25519-dalek` directly — every crypto call goes through
`syneroym_identity::substrate`, which is the point of `D-C3-6`.

### 3.2 `crates/signed_record/src/lib.rs`

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The signed record envelope: one stable byte encoding, one verifier, and
//! the rules the host applies before it signs anything.
//!
//! Compiles for the host and for `wasm32-wasip2`. Deliberately exposes no
//! function taking a `syneroym_identity::Identity`: producing a signature
//! needs a private key, and the only private key involved lives on the far
//! side of the `syneroym:signing` WIT boundary. A component linking this
//! crate can build a draft and verify an envelope; it cannot sign one.

pub mod envelope;
pub mod verify;

pub use envelope::{
    DraftError, Envelope, EnvelopeError, RecordDraft, ENVELOPE_VERSION, MAX_PAYLOAD_BYTES,
    MAX_PAYLOAD_DEPTH, MAX_RECORD_TYPE_LEN, MAX_SUBJECT_LEN, RECORD_ID_PREFIX,
};
pub use syneroym_identity::delegation::SCOPE_RECORD_SIGNING;
pub use verify::{
    verify, verify_json, DEFAULT_ACCEPTED_SCOPES, EmptyRevocations, RevocationSet,
    RevocationSource, VerifiedRecord, VerifyError, VerifyOptions,
};
```

### 3.3 `crates/signed_record/src/envelope.rs`

```rust
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_identity::{delegation::DelegationCertificate, substrate};

/// The shape of this struct. Bumped only when a field is added, removed, or
/// reinterpreted -- which changes the canonical bytes and so invalidates
/// every signature already produced. A verifier that does not know the
/// value refuses the record; it never guesses and never migrates.
pub const ENVELOPE_VERSION: u32 = 1;

pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_PAYLOAD_DEPTH: usize = 32;
pub const MAX_RECORD_TYPE_LEN: usize = 64;
pub const MAX_SUBJECT_LEN: usize = 256;
pub const RECORD_ID_PREFIX: &str = "rec_";
```

**`RecordDraft` — everything the guest supplies, and nothing else.**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordDraft {
    pub version: u32,
    pub record_type: String,
    pub subject: String,
    pub payload: Value,
    pub expires_at_secs: Option<u64>,
    pub supersedes: Option<String>,
}
```

**`DraftError` — the refusal list of `D-C3-11`, one variant per rule.**

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    #[error("a record with version 0 does not exist")]
    ZeroVersion,
    #[error("record type '{0}' is not lowercase ascii letters, digits and '-', 1..={max} bytes", max = MAX_RECORD_TYPE_LEN)]
    RecordType(String),
    #[error("subject is {0} bytes, over the {max}-byte maximum", max = MAX_SUBJECT_LEN)]
    Subject(usize),
    #[error("payload is not a JSON object")]
    PayloadNotObject,
    #[error("payload is {bytes} bytes, over the {max}-byte maximum")]
    PayloadTooLarge { bytes: usize, max: usize },
    #[error("payload nests {depth} deep, over the {max} maximum")]
    PayloadTooDeep { depth: usize, max: usize },
    /// The canonical encoding is only reproducible for integers -- see the
    /// module note on `canonical_bytes`.
    #[error("payload field '{0}' holds a number that is not an integer")]
    PayloadNonIntegerNumber(String),
    #[error("supersedes '{0}' is not a record id")]
    Supersedes(String),
    #[error("expires_at_secs {expires_at_secs} is already past (now {now_secs})")]
    ExpiryInPast { expires_at_secs: u64, now_secs: u64 },
}
```

```rust
impl RecordDraft {
    /// Every rule the host applies before it will build an envelope. Pure
    /// and clock-parameterised so the same list is testable without a
    /// substrate.
    pub fn validate(&self, now_secs: u64) -> Result<(), DraftError> { … }
}
```

Pseudo-code for `validate`:

```
if self.version == 0                      -> ZeroVersion
if record_type is empty
   or len > MAX_RECORD_TYPE_LEN
   or any byte outside [a-z0-9-]           -> RecordType
if subject.len() > MAX_SUBJECT_LEN         -> Subject
match &self.payload { Value::Object(_) => (), _ => return PayloadNotObject }
walk(payload, depth = 1, path = ""):
    if depth > MAX_PAYLOAD_DEPTH           -> PayloadTooDeep
    for each Value::Number n:
        if !n.is_i64() && !n.is_u64()      -> PayloadNonIntegerNumber(path)
    recurse into objects and arrays
bytes = serde_json::to_vec(canonicalize(&payload)).len()
if bytes > MAX_PAYLOAD_BYTES               -> PayloadTooLarge
if let Some(s) = &self.supersedes:
    if !s.starts_with(RECORD_ID_PREFIX)
       or z32::decode(&s[4..]) is Err
       or decoded.len() != 32              -> Supersedes
if let Some(e) = self.expires_at_secs:
    if e <= now_secs                       -> ExpiryInPast
Ok(())
```

**`Envelope` — the record on the wire and on disk.**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// The shape of *this struct*. No serde default: an envelope without
    /// it fails to parse rather than assuming the current shape.
    pub envelope_version: u32,
    /// The record *body*'s own schema version, chosen by whatever produced
    /// `payload`. No serde default: a record with no version fails to
    /// parse rather than defaulting to one.
    pub version: u32,
    pub record_type: String,
    /// The DID this record is asserted under. Self-declared, and proves
    /// nothing on its own -- a verifier checks it against an issuer it
    /// already decided to trust (`VerifyOptions::expected_issuer`), never
    /// the other way round. A directory or any other third party asserting
    /// this field is true carries no weight; only the signature under it
    /// does.
    pub issuer: String,
    /// A DID, another record's id, or "" when the body is the whole
    /// subject.
    pub subject: String,
    pub issued_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_secs: Option<u64>,
    /// The `record_id` of the record this one corrects. Nothing here is
    /// ever edited in place: a correction is always a new record, and both
    /// the old and the new one survive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Always a JSON object.
    pub payload: Value,
    /// JSON `DelegationCertificate` tying the key that produced
    /// `signature` to `issuer`. Inside the signed payload, so it cannot be
    /// swapped for another master's certificate. `None` means `issuer`
    /// signed with its own key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<String>,
    /// z-base-32 Ed25519 signature over `signing_bytes()`.
    pub signature: String,
}
```

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("envelope json: {0}")]
    Json(String),
    #[error("signature already attached")]
    AlreadySigned,
}

impl Envelope {
    /// Builds the unsigned envelope and the exact bytes its signature must
    /// cover. Host-only: `issuer`, `delegation` and `issued_at_secs` come
    /// from the substrate, never from `draft`.
    pub fn unsigned(
        draft: RecordDraft,
        issuer: String,
        delegation: Option<String>,
        issued_at_secs: u64,
    ) -> Result<(Self, Vec<u8>), DraftError>;

    /// Fills in `signature`. `AlreadySigned` if it is not empty.
    pub fn attach_signature(&mut self, signature_z32: String) -> Result<(), EnvelopeError>;

    /// The bytes `signature` covers: this struct with `signature` set to
    /// "", key-sorted and serialized. Mirrors `Identity::sign_json`'s
    /// convention exactly (`RelationshipProof` sets the same precedent) --
    /// never a bespoke scheme.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, EnvelopeError>;

    /// Content-derived id, covering every field including `signature`.
    /// `rec_` + z32(sha256(canonical bytes of self)). Deterministic:
    /// byte-identical envelopes (same draft, same key, same delegation,
    /// same `issued_at_secs`) always derive the same id -- a retried sign
    /// converges on the same record rather than minting a second one.
    pub fn record_id(&self) -> Result<String, EnvelopeError>;

    pub fn to_json(&self) -> Result<String, EnvelopeError>;
    pub fn from_json(s: &str) -> Result<Self, EnvelopeError>;
}
```

`unsigned` pseudo-code:

```
draft.validate(issued_at_secs)?
Envelope {
    envelope_version: ENVELOPE_VERSION,
    version: draft.version,
    record_type: draft.record_type,
    issuer,
    subject: draft.subject,
    issued_at_secs,
    expires_at_secs: draft.expires_at_secs,
    supersedes: draft.supersedes,
    payload: substrate::canonicalize_json_value(&draft.payload),
    delegation,
    signature: String::new(),
}
-> (envelope, envelope.signing_bytes()?)
```

`signing_bytes` pseudo-code:

```
let mut unsigned = self.clone();
unsigned.signature = String::new();
let value = serde_json::to_value(&unsigned)?;
serde_json::to_vec(&substrate::canonicalize_json_value(&value))
```

`record_id` pseudo-code:

```
let value = serde_json::to_value(self)?;
let bytes = serde_json::to_vec(&substrate::canonicalize_json_value(&value))?;
format!("{RECORD_ID_PREFIX}{}", z32::encode(&Sha256::digest(&bytes)))
```

> **Note in the module doc, not just here.** `canonicalize_json_value` is
> key-sorting plus `serde_json`, not full RFC 8785 (F3). The
> non-integer-number refusal in `RecordDraft::validate` is what makes these
> bytes reproducible; do not relax it without replacing the
> canonicalization.

### 3.4 `crates/signed_record/src/verify.rs`

```rust
/// A per-check verdict, never a bare `bool`: `Unknown` is a distinct
/// outcome from `Good`, and a source with nothing to say must return it
/// rather than defaulting to "not revoked". A `bool`-shaped answer cannot
/// tell "checked, clean" apart from "never checked", which is exactly the
/// distinction a search result or a cached credential needs to render
/// correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationCheck {
    Good,
    Revoked,
    Unknown,
}

pub trait RevocationSource {
    /// `did` is a signing key or an issuer DID. Never a network call: a
    /// verifier that asks a directory whether a record is good has handed
    /// the directory the verdict, which is exactly what this crate exists
    /// to avoid -- a directory's assertion is evidence to go fetch a real
    /// revocation list over, never a substitute for checking one.
    fn check_did(&self, did: &str) -> RevocationCheck;
    /// Has a `revocation` record been seen against this record id.
    fn check_record(&self, record_id: &str) -> RevocationCheck;
}

/// Nothing is known either way. The honest default for a node that has not
/// fetched a revocation list: every check answers `Unknown`, never `Good`
/// -- a caller that forgets to wire a real source gets "unknown" on every
/// record, not a silent "not revoked".
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyRevocations;

/// An in-memory set of *known-revoked* entries -- absence from either set
/// still means `Unknown`, not `Good`; this type has no way to assert
/// cleanliness, only revocation. C9 supplies the real source; this crate
/// supplies the shape and proves every branch.
#[derive(Debug, Clone, Default)]
pub struct RevocationSet {
    pub revoked_dids: std::collections::BTreeSet<String>,
    pub revoked_records: std::collections::BTreeSet<String>,
}

pub const DEFAULT_ACCEPTED_SCOPES: &[&str] = &[SCOPE_RECORD_SIGNING];

pub struct VerifyOptions<'a> {
    pub now_secs: u64,
    /// The issuer the caller decided to trust **before** it fetched this
    /// record. `None` skips the check and hands the caller the
    /// responsibility -- `Envelope::issuer` is self-declared.
    pub expected_issuer: Option<&'a str>,
    pub accepted_scopes: &'a [&'a str],
    pub revoked: &'a dyn RevocationSource,
    /// How far into the future `issued_at_secs` may sit before the record
    /// is refused.
    pub max_clock_skew_secs: u64,
}

impl<'a> VerifyOptions<'a> {
    /// `expected_issuer: None`, `DEFAULT_ACCEPTED_SCOPES`,
    /// `EmptyRevocations`, 300s skew.
    pub fn new(now_secs: u64) -> VerifyOptions<'static>;
    pub fn expecting(self, issuer: &'a str) -> Self;
    pub fn with_revocations(self, src: &'a dyn RevocationSource) -> Self;
}

/// The overall revocation verdict a verified record carries. Never
/// `Revoked` here: a `Revoked` verdict from `RevocationSource` is a hard
/// `VerifyError` and never reaches a `VerifiedRecord` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationStatus {
    /// Every check that ran came back clean.
    Good,
    /// At least one check came back `Unknown`. The caller must render this
    /// as unknown, never as a positive default.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecord {
    pub record_id: String,
    pub issuer: String,
    /// The key that actually produced the signature: the issuer itself, or
    /// the delegated key the certificate names.
    pub signer_did: String,
    pub record_type: String,
    pub version: u32,
    pub subject: String,
    pub payload: Value,
    pub issued_at_secs: u64,
    pub expires_at_secs: Option<u64>,
    pub supersedes: Option<String>,
    pub revocation_status: RevocationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("malformed record: {0}")]
    Malformed(String),
    #[error("envelope version {0} is not understood by this build")]
    UnknownEnvelopeVersion(u32),
    #[error("issuer mismatch: expected '{expected}', got '{actual}'")]
    IssuerMismatch { expected: String, actual: String },
    #[error("record expired at {expires_at_secs} (now {now_secs})")]
    Expired { now_secs: u64, expires_at_secs: u64 },
    #[error("record is dated {issued_at_secs}, in the future (now {now_secs})")]
    IssuedInFuture { now_secs: u64, issued_at_secs: u64 },
    #[error("delegation is invalid: {0}")]
    BadDelegation(String),
    #[error("signature is invalid: {0}")]
    BadSignature(String),
    #[error("signing key '{0}' is revoked")]
    RevokedKey(String),
    #[error("record '{0}' is revoked")]
    RevokedRecord(String),
}

pub fn verify(e: &Envelope, o: &VerifyOptions<'_>) -> Result<VerifiedRecord, VerifyError>;
pub fn verify_json(json: &str, o: &VerifyOptions<'_>) -> Result<VerifiedRecord, VerifyError>;
```

`verify` pseudo-code — **the order is part of the contract**; cheap and
non-cryptographic checks first, so a caller's error tells it what is wrong
rather than always saying "bad signature":

```
1. if e.envelope_version != ENVELOPE_VERSION -> UnknownEnvelopeVersion
2. structural: RecordDraft-equivalent shape checks on version /
   record_type / subject / payload (reuse RecordDraft::validate with
   expires bypassed -- an already-expired record must fail at step 4 with
   Expired, not at step 2 with Malformed)  -> Malformed
3. if let Some(want) = o.expected_issuer, and want != e.issuer
                                             -> IssuerMismatch
4. if e.issued_at_secs > o.now_secs + o.max_clock_skew_secs
                                             -> IssuedInFuture
   if let Some(exp) = e.expires_at_secs, and o.now_secs >= exp
                                             -> Expired
5. signer_did = match &e.delegation {
       Some(json) => {
           let cert = DelegationCertificate::from_json(json)
                          .map_err(BadDelegation)?;
           // `verify_chain`, not `verify`. `verify_chain` checks the
           // certificate's own signature, that its master matches
           // `e.issuer`, and its scope -- but deliberately NOT its
           // wall-clock expiry: a record does not retroactively stop
           // having been validly signed on the day its signing
           // certificate happens to expire. What must hold instead is
           // that the certificate was valid AT THE MOMENT this record was
           // signed, checked explicitly below against the record's own
           // `issued_at_secs` rather than against `now_secs`. Using
           // `verify` here (wall-clock expiry) would make a record's
           // verdict silently flip from valid to invalid on every future
           // verification once the certificate's short window passes,
           // with no way to tell "this was never valid" apart from "this
           // was valid once and the record has outlived the paperwork".
           cert.verify_chain(&e.issuer, o.accepted_scopes)
                          .map_err(BadDelegation)?;
           if e.issued_at_secs < cert.issued_at_secs
              || e.issued_at_secs >= cert.expires_at_secs {
               return Err(BadDelegation(format!(
                   "record issued at {}, outside the certificate's own \
                    window [{}, {})",
                   e.issued_at_secs, cert.issued_at_secs, cert.expires_at_secs)));
           }
           cert.temporary_did
       }
       None => e.issuer.clone(),
   }
6. did_check = worse(o.revoked.check_did(&signer_did),
                      o.revoked.check_did(&e.issuer))
   if did_check == Revoked -> RevokedKey(whichever did was flagged)
7. substrate::verify_json_signature(&signer_did,
        &serde_json::from_slice(&e.signing_bytes()?)?, &e.signature)
                                              -> BadSignature
8. record_id = e.record_id()?;
   record_check = o.revoked.check_record(&record_id)
   if record_check == Revoked -> RevokedRecord(record_id)
9. revocation_status = if did_check == Unknown || record_check == Unknown
                            { RevocationStatus::Unknown }
                        else { RevocationStatus::Good }
   Ok(VerifiedRecord { …, revocation_status })
```

(`worse(a, b)`: `Revoked` beats `Unknown` beats `Good` -- the stricter of
the two verdicts wins, so one revoked DID out of two checked is still a
hard failure.)

Step 5 is where finding is kept structurally separate from trusting: the
certificate travels inside the signed bytes, so a directory cannot
substitute one, and `cert.verify_chain` re-checks the scope so a
`service-instance` certificate can never stand in for a `record-signing`
one. Step 9 is where the same separation reaches the caller: an `Unknown`
revocation status is returned alongside a fully verified signature, never
silently upgraded to `Good` and never turned into a hard failure on its
own -- what the caller does with "the signature is good but I don't know
if it's revoked" is the caller's decision to render, not this function's
to make for it.

### 3.5 Unit tests in `crates/signed_record`

One test per `DraftError` variant and one per `VerifyError` variant, plus:

- `sign_then_verify_round_trips` (uses a locally generated `Identity` in a
  `#[cfg(test)]` helper — the crate's own tests may use `Identity`; its
  public API may not).
- `a_tampered_payload_field_fails_the_signature`.
- `a_swapped_delegation_fails_because_it_is_inside_the_signed_bytes`.
- `a_service_instance_scoped_certificate_is_refused_as_a_record_delegation`.
- `record_id_is_stable_across_serialize_deserialize`.
- `record_id_changes_when_any_field_changes` (walk every field).
- `an_envelope_without_version_fails_to_parse` and the same for
  `envelope_version` -- a record without one is refused rather than
  defaulting to a value.
- `a_float_in_the_payload_is_refused` and
  `an_integer_valued_float_is_still_refused` (`1.0` parses as `f64`; it is
  refused, and the message says so — the alternative, silently accepting
  it, is what makes canonical bytes irreproducible).
- **The delegation-window regression class, added after review found the
  original design checked the wrong clock:**
  - `a_record_signed_under_a_still_valid_certificate_verifies_after_that_certificate_has_since_wall_clock_expired`
    — sign at `t0` under a certificate valid `[t0 - 60, t0 + 3600)`; verify
    with `now_secs = t0 + 100_000` (long past the certificate's own
    `expires_at_secs`, but the record itself carries no `expires_at_secs`
    of its own) — must still succeed. This is the test that would have
    caught the original bug: it fails under `cert.verify(...)` and passes
    under `cert.verify_chain(...)` plus the explicit window check.
  - `a_record_dated_before_the_certificate_was_issued_is_refused`.
  - `a_record_dated_at_or_after_the_certificate_expires_is_refused`.
- **Revocation, tri-state:**
  - `an_unknown_did_and_an_unknown_record_verify_as_good_with_unknown_revocation_status`
    (`EmptyRevocations`) — the signature check still runs and passes; only
    `revocation_status` says `Unknown`.
  - `a_revoked_signing_key_is_a_hard_verify_error`, same for a revoked
    issuer DID and a revoked record id, each via `RevocationSet`.
  - `a_known_clean_did_and_record_verify_as_good_with_good_revocation_status`.

---

## §4 `syneroym-identity` — one constant and three warnings

### 4.1 `crates/identity/src/delegation.rs`, after `SCOPE_SERVICE_INSTANCE`

```rust
/// A person's master key delegating to a substrate-derived service signing
/// key, for signing product records under that person's DID. Deliberately
/// **not** in `TRANSPORT_SCOPES`: a certificate minted to sign records must
/// never be replayable onto a connection preamble, and must never stand in
/// for an ADR-0020 instance certificate.
pub const SCOPE_RECORD_SIGNING: &str = "record-signing";
```

`TRANSPORT_SCOPES` is unchanged. Add a test asserting
`!TRANSPORT_SCOPES.contains(&SCOPE_RECORD_SIGNING)`.

### 4.2 `crates/identity/src/keys.rs` — the three `wasm32` warnings (F10)

`mise run build:roym` and any wasm build of `syneroym-signed-record` will
surface them. Gate the `io` import and `lock_memory`'s parameters on the
same `cfg` the `mlock` body already uses.

---

## §5 The WIT surface — `syneroym:signing@0.1.0`

### 5.1 New file `crates/wit_interfaces/wit/signing/signing.wit`

```wit
package syneroym:signing@0.1.0;

/// Signing a product record under a principal whose key this substrate
/// holds. No function here returns key material, and there is no
/// "sign these bytes" verb: the host builds the envelope it signs, so it
/// can refuse one it cannot parse, and a component can never state its own
/// issuer or its own timestamps.
interface signing {
    /// Which principal a record is asserted under.
    variant principal {
        /// The service itself, signing with the key this substrate derives
        /// for it. The issuer is that key's `did:key`.
        service,
        /// Any other master DID -- a natural person, or an organisation's
        /// own dedicated identity -- proven by a JSON
        /// `DelegationCertificate` that master key minted over *this
        /// service's own* signing key, scoped `record-signing`. The host
        /// re-checks the certificate on every call and refuses any that
        /// does not certify the exact key it is about to sign with -- so a
        /// component may hold a certificate (it carries no private key)
        /// and still cannot invent an issuer. When the caller itself
        /// arrived as a verified identity (a session, not an internal
        /// substrate-injected context), the host also requires the
        /// certificate's master to match that caller -- holding someone
        /// else's certificate is not the same as being them. Get the key
        /// to certify from `identity` below.
        delegated(string),
    }

    variant signing-error {
        /// `principal::delegated` was asked for and the certificate is
        /// missing, malformed, scoped for something else, certifies a
        /// different key, does not cover the record's own timestamp, or
        /// names a master the calling session does not match.
        no-delegation(string),
        /// The draft is not a record this host will build an envelope
        /// around. The string names the rule that refused it.
        invalid-record(string),
        /// This instance may not sign at all -- a read-only after-step
        /// instance, today.
        permission-denied,
        internal(string),
    }

    /// Everything the component supplies. Issuer, issued-at, the
    /// delegation and the signature are the host's, and are not
    /// expressible here.
    record record-draft {
        /// The record body's own schema version. Never 0.
        version: u32,
        /// `listing`, `quote`, ... Lowercase ASCII letters, digits and
        /// `-`, 1 to 64 bytes. The host checks the shape, not the
        /// vocabulary: which types exist is the app's to decide.
        record-type: string,
        /// What the record is about -- a DID, another record's id, or the
        /// empty string when the body is the whole subject.
        subject: string,
        /// The record body, as a JSON **object**. Refused if it is not an
        /// object, if it is over 64 KiB canonicalized, if it nests deeper
        /// than 32, or if it holds a number that is not an integer -- the
        /// canonical encoding is only reproducible for integers, so a
        /// price is minor units, not a decimal.
        payload: string,
        /// Unix seconds after which this record must not be treated as
        /// valid. Absent means it does not expire on its own.
        expires-at-secs: option<u64>,
        /// The `record-id` of the record this one corrects. A correction is
        /// always a new record; nothing is ever edited.
        supersedes: option<string>,
    }

    /// Builds the canonical envelope around `draft`, signs it as
    /// `as-principal`, and returns the whole signed envelope as JSON.
    sign-record: func(
        draft: record-draft,
        as-principal: principal,
    ) -> result<string, signing-error>;

    /// Public identifiers only.
    record signing-identity {
        /// `did:key` of the key this substrate signs this service's
        /// records with.
        signing-did: string,
        /// The same key, hex-encoded -- the form
        /// `orchestrator/resolve-instance-identity` reports and a
        /// certificate is minted over.
        pubkey-hex: string,
        /// The DID recorded as this service's deployer. Whoever holds that
        /// key is the one who can mint a `record-signing` certificate this
        /// host will accept. Absent for a service with no recorded owner.
        owner-did: option<string>,
    }

    /// What this service can sign as, right now.
    identity: func() -> result<signing-identity, signing-error>;
}

/// One world, used by both bindgen passes: this package has no exports, so
/// the guest view and the host view are the same shape. `proxy-import` and
/// `conversation-import` needed a second world only because their packages
/// also declare guest exports.
world signing-import {
    import signing;
}
```

### 5.2 `crates/wit_interfaces/src/signing.rs` — new file

```rust
//! Guest-side bindings for `syneroym:signing`.

wit_bindgen::generate!({
    world: "signing-import",
    path: "wit/signing/signing.wit",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
```

Downstream path:
`syneroym_wit_interfaces::signing::syneroym::signing::signing::{sign_record, identity, Principal, RecordDraft, SigningError, SigningIdentity}`.

### 5.3 `crates/wit_interfaces/src/signing_host.rs` — new file

```rust
//! Host-side bindings for `syneroym:signing`. A separate `bindgen!` rather
//! than a `host-environment` import, so a component that does not import
//! `signing` deploys exactly as before -- the same reason
//! `conversation_host` is separate.

wasmtime::component::bindgen!({
    path: "wit/signing",
    world: "signing-import",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
});
```

### 5.4 `crates/wit_interfaces/src/lib.rs`

```rust
 #[cfg(feature = "proxy")]
 pub mod proxy;
+#[cfg(feature = "signing")]
+pub mod signing;
 #[cfg(feature = "supervisor")]
 pub mod supervisor;
…
 #[cfg(not(target_arch = "wasm32"))]
 pub mod conversation_host;
+
+#[cfg(not(target_arch = "wasm32"))]
+pub mod signing_host;
```

### 5.5 `crates/wit_interfaces/Cargo.toml`

```toml
default = ["app-config", "blob-store", "control-plane", "conversation", "data-layer", "http", "messaging", "proxy", "signing", "supervisor", "vault"]
…
proxy = []
signing = []
supervisor = []
```

`host.wit` is **not** touched, and no `wit/host/deps/signing` symlink is
created — `host-environment` does not import `signing` (`D-C3-1`).

---

## §6 `syneroym-app-host` — the trait and the guest bridge

### 6.1 `Cargo.toml`

Add `"signing"` to the `syneroym-wit-interfaces` feature list (keep it
alphabetical, between `proxy` and `vault`).

### 6.2 `src/types.rs` — one new module

```rust
pub mod signing {
    pub use syneroym_wit_interfaces::signing::syneroym::signing::signing::{
        Principal, RecordDraft, SigningError, SigningIdentity,
    };
}
```

Placed after `pub mod proxy`, before `pub mod app_config`, matching the
file's existing order.

### 6.3 `src/lib.rs` — the trait and the supertrait

```rust
/// Mirrors `syneroym:signing/signing@0.1.0`, function for function.
pub trait AppSigning {
    fn sign_record(
        &self,
        draft: RecordDraft,
        as_principal: Principal,
    ) -> impl Future<Output = Result<String, SigningError>> + Send;

    fn signing_identity(
        &self,
    ) -> impl Future<Output = Result<SigningIdentity, SigningError>> + Send;
}
```

`AppHost` and its blanket impl each gain `+ AppSigning` after `AppVault`:

```diff
 pub trait AppHost:
     AppDataLayer
     + AppBlobStore
     + AppMessaging
     + AppConversation
     + AppProxy
     + AppAppConfig
     + AppVault
+    + AppSigning
     + AppWebSocket
     + Send
     + Sync
 {
 }
-impl<T> AppHost for T where T: … + AppVault + AppWebSocket + Send + Sync {}
+impl<T> AppHost for T where T: … + AppVault + AppSigning + AppWebSocket + Send + Sync {}
```

**This is a breaking change to `AppHost`'s bound**, exactly as `task.md`'s
Migration-impact section anticipates. There are two implementors, both
in-tree: `GuestHost` (§6.4) and `NativeAppHost` (§8.2). Nothing else needs
touching — every consumer is generic over `H: AppHost`.

Add `signing::{Principal, RecordDraft, SigningError, SigningIdentity}` to
the `use types::{…}` block at the top of the file.

### 6.4 `src/guest.rs`

```rust
use syneroym_wit_interfaces::{
    …,
    signing::syneroym::signing::signing as sgn,
    …
};

impl AppSigning for GuestHost {
    async fn sign_record(
        &self,
        draft: RecordDraft,
        as_principal: Principal,
    ) -> Result<String, SigningError> {
        sgn::sign_record(&draft, &as_principal)
    }

    async fn signing_identity(&self) -> Result<SigningIdentity, SigningError> {
        sgn::identity()
    }
}
```

Placed after `impl AppVault for GuestHost`. Confirm the generated
`sign_record` argument convention (`&RecordDraft` / `&Principal` vs by
value) against the emitted bindings before writing this — `wit-bindgen`
passes `record`s and `variant`s with payloads by reference.

---

## §7 `syneroym-core` — `NodeRecordSigner`

### 7.1 `Cargo.toml`

```toml
syneroym-signed-record.workspace = true
```

### 7.2 New file `crates/core/src/record_signer.rs`

```rust
//! The one place a product record is signed. Holds the node identity and
//! the endpoint registry, derives the per-service signing key the same way
//! `SynSvcNativeService` and `resolve-instance-identity` already do, and
//! applies every refusal rule before it touches the key.
//!
//! Concrete and held `Arc`, not `Weak<dyn …>`: unlike `ConversationHost`
//! and `ServiceProxy` this holds no reference back to the sandbox engine or
//! the native factory, so there is no cycle to guard against.

use std::sync::Arc;

use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::derive_did_key,
};
use syneroym_signed_record::{DraftError, Envelope, RecordDraft};

use crate::local_registry::EndpointRegistry;

/// Which principal a record is asserted under. Mirrors
/// `syneroym:signing/signing.principal`; plain Rust here for the same
/// reason `syneroym_rpc::conversation`'s types are plain Rust -- this crate
/// has no `wasmtime` and no `syneroym-wit-interfaces`, and each host
/// implementation converts at its own boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigningPrincipal {
    Service,
    Delegated { delegation_json: String },
}

/// The calling identity, as far as this signer needs to know it -- a
/// deliberately narrow mirror of `syneroym_rpc::AuthLevel`. `AuthLevel`
/// is not used directly here because depending on `syneroym-rpc` would
/// drag `syneroym-wit-interfaces` and `wasmtime` into `syneroym-core`,
/// which is a transitive-weight problem, not a cycle: neither
/// `syneroym-rpc` nor `syneroym-core` depends on the other today
/// (`crates/rpc/Cargo.toml` lists `ucan`, `fdae`, `identity`, `app-host`;
/// `crates/core/Cargo.toml` lists only `identity`). The narrow enum is
/// the right choice; state the right reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerBinding<'a> {
    /// An externally verified caller claim -- a session token or a UCAN
    /// chain -- naming this DID. The strongest identity information this
    /// signer ever sees.
    Verified(&'a str),
    /// A substrate-injected context with no externally verified DID to
    /// check a certificate against: a cross-service sibling call via
    /// `service_system`, a lifecycle hook, a stage-4 after-step.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    #[error("no usable delegation: {0}")]
    NoDelegation(String),
    #[error("refused: {0}")]
    InvalidRecord(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningIdentity {
    pub signing_did: String,
    pub pubkey_hex: String,
    pub owner_did: Option<String>,
}

/// Where `issued_at_secs` comes from. `Fixed` exists so the dual-build
/// parity suite can assert the two builds produce byte-identical envelopes
/// -- ed25519 signing in this tree is deterministic, so the wall clock is
/// the only thing that would differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordClock {
    System,
    Fixed(u64),
}

#[derive(Debug)]
pub struct NodeRecordSigner {
    node_identity: Arc<Identity>,
    registry: EndpointRegistry,
    clock: RecordClock,
}
```

```rust
impl NodeRecordSigner {
    pub fn new(node_identity: Arc<Identity>, registry: EndpointRegistry) -> Arc<Self>;
    pub fn with_clock(node_identity: Arc<Identity>, registry: EndpointRegistry, clock: RecordClock) -> Arc<Self>;

    pub fn identity(&self, service_id: &str) -> SigningIdentity;

    pub fn sign_record(
        &self,
        service_id: &str,
        draft: RecordDraft,
        principal: &SigningPrincipal,
        caller: CallerBinding<'_>,
    ) -> Result<String, SigningError>;
}
```

Private helper, and the one rule the whole slice turns on:

```rust
    /// The key this node signs `service_id`'s records with. Identical to
    /// what `SynSvcNativeService` derives and to what
    /// `orchestrator/resolve-instance-identity` reports for the recorded
    /// owner -- three call sites, one derivation, or a minted certificate
    /// stops matching the key it was minted for.
    ///
    /// A service with no recorded owner (a natively linked app, which has
    /// no deploy record) derives under the node's own DID. Deterministic
    /// and documented rather than clever: if such a service later gains a
    /// recorded owner, its signing key changes and records signed before
    /// that no longer verify under the new DID. `[roles.roym] owner_did`
    /// (§11.2) exists so a natively linked deployment states its owner up
    /// front instead of drifting into that.
    fn service_identity(&self, service_id: &str) -> Identity {
        let owner = self
            .registry
            .owner_of(service_id)
            .unwrap_or_else(|| derive_did_key(&self.node_identity.public_key()));
        self.node_identity.derive_service_identity(&owner, service_id)
    }
```

`sign_record` pseudo-code:

```
let now = match self.clock { System => unix_now(), Fixed(t) => t };
let key = self.service_identity(service_id);
let key_did = derive_did_key(&key.public_key());

let (issuer, delegation) = match principal {
    Service => (key_did.clone(), None),
    Delegated { delegation_json } => {
        let cert = DelegationCertificate::from_json(delegation_json)
            .map_err(|e| NoDelegation(format!("not a delegation certificate: {e}")))?;
        // (1) it certifies the key we are about to sign with -- the check
        //     that makes a guest-held certificate safe on its own.
        if cert.temporary_did != key_did {
            return Err(NoDelegation(format!(
                "certificate certifies '{}', not this service's signing key '{key_did}'",
                cert.temporary_did)));
        }
        // (2) signature, wall-clock window, and the narrow scope, checked
        //     right now because signing under an already-expired
        //     delegation would mint a record that never had a valid
        //     window to begin with. `verify`, not `verify_chain`: unlike
        //     the *verifier* side (§3.4), which must accept a record
        //     signed while the certificate was still good even after that
        //     certificate has since expired, the *signer* is deciding
        //     whether to sign right now, and "right now" is exactly what
        //     wall-clock `verify` checks.
        cert.verify(&cert.master_did, &[SCOPE_RECORD_SIGNING])
            .map_err(|e| NoDelegation(e.to_string()))?;
        // (3) holding the certificate is not the same as being the master
        //     it names. When this call arrived as a verified identity --
        //     a live session or UCAN chain -- that identity must be the
        //     certificate's own master, or the guest is presenting
        //     someone else's certificate over a session that is not
        //     theirs. A substrate-injected caller (`Internal`) has no
        //     externally verified DID to hold this check against and is
        //     not subject to it -- narrowing that residual is a capability
        //     gate this slice does not build (tracked separately).
        if let CallerBinding::Verified(caller_did) = caller
            && caller_did != cert.master_did
        {
            return Err(NoDelegation(format!(
                "certificate names master '{}', which does not match the calling \
                 session's subject '{caller_did}'",
                cert.master_did)));
        }
        (cert.master_did.clone(), Some(delegation_json.clone()))
    }
};

let (mut env, bytes) = Envelope::unsigned(draft, issuer, delegation, now)
    .map_err(|e: DraftError| InvalidRecord(e.to_string()))?;
env.attach_signature(z32::encode(&key.sign(&bytes).to_bytes()))
    .map_err(|e| Internal(e.to_string()))?;
env.to_json().map_err(|e| Internal(e.to_string()))
```

The check at (2) is sound only because the issuer is then taken **from the
certificate**, never from the caller: the caller is choosing whose name to
sign under, and the certificate plus (3)'s binding check together are the
proof it may. The verifier's own expected-issuer check (§3.4 step 3) is a
separate, later check by a different party and does not duplicate this
one.

`identity` pseudo-code:

```
let key = self.service_identity(service_id);
SigningIdentity {
    signing_did: derive_did_key(&key.public_key()),
    pubkey_hex: hex::encode(key.public_key().to_bytes()),
    owner_did: self.registry.owner_of(service_id),
}
```

### 7.3 `crates/core/src/lib.rs`

```rust
+pub mod record_signer;
```

### 7.4 Unit tests in `record_signer.rs`

- `a_service_principal_signs_under_the_derived_key_and_verifies`.
- `the_derived_key_matches_what_resolve_instance_identity_reports`
  (construct `Identity::derive_service_identity(owner, sid)` by hand and
  compare) — the regression guard for the three-call-site derivation
  (§8.3 threads the *same* `NodeRecordSigner` to both
  `SynSvcNativeService::new` call sites; this test only proves the
  derivation formula, not that threading — see §13.3 for that).
- `a_delegated_certificate_over_another_key_is_refused`.
- `a_service_instance_scoped_certificate_is_refused_as_a_delegation`.
- `an_already_expired_delegated_certificate_is_refused_at_signing_time`
  (distinct from the verifier's own window test in §3.5: this one proves
  the *signer* still uses wall-clock `verify`, not `verify_chain`).
- `a_verified_caller_whose_did_does_not_match_the_certificates_master_is_refused`.
- `an_internal_caller_presenting_any_valid_certificate_is_not_checked_against_a_caller_did`
  (`CallerBinding::Internal` — the residual §16 tracks, proven rather than
  assumed).
- `a_float_payload_is_refused_as_invalid_record`.
- `two_signers_with_the_same_node_identity_and_owner_produce_identical_bytes`
  (`RecordClock::Fixed`) — the property the parity suite depends on.

---

## §8 The host implementations

### 8.1 `syneroym-sandbox-wasm` — `HostState` gains the capability

**`crates/sandbox_wasm/src/host_capabilities.rs`**

1. New field on `HostState` (after `websocket_senders`):

```rust
    /// The node's record signer (`syneroym:signing`). `Option`, not
    /// `Weak`: it holds no path back to this engine, so there is no cycle,
    /// and `None` is the honest state for a node that never wired one
    /// (every existing `HostState::new` call site, all of them tests).
    pub record_signer: Option<Arc<NodeRecordSigner>>,
```

Default to `None` inside `HostState::new`'s struct literal
(`host_capabilities.rs:344`), and add the setter beside
`with_conversation` (`:352`):

```rust
    #[must_use]
    pub fn with_record_signer(mut self, signer: Option<Arc<NodeRecordSigner>>) -> Self {
        self.record_signer = signer;
        self
    }
```

2. The `Host` impl, placed after `impl vault::Host for HostState`
(`:501`):

```rust
impl signing::Host for HostState {
    async fn sign_record(
        &mut self,
        draft: WitRecordDraft,
        as_principal: WitPrincipal,
    ) -> Result<String, WitSigningError> {
        if self.read_only {
            return Err(WitSigningError::PermissionDenied);
        }
        let Some(signer) = self.record_signer.clone() else {
            return Err(WitSigningError::Internal(
                "this node has no record signer configured".to_string(),
            ));
        };
        let draft = convert::draft_in(draft)?;   // parses `payload` JSON
        let principal = convert::principal_in(as_principal);
        let caller = caller_binding(&self.caller);
        signer
            .sign_record(&self.component_id, draft, &principal, caller)
            .map_err(convert::signing_error_out)
    }

    async fn identity(&mut self) -> Result<WitSigningIdentity, WitSigningError> {
        let Some(signer) = self.record_signer.clone() else { … };
        Ok(convert::identity_out(signer.identity(&self.component_id)))
    }
}

/// `HostState.caller: CallerContext` -> `NodeRecordSigner`'s narrower
/// `CallerBinding`. `AuthLevel::Delegated`/`Ucan` are the two shapes an
/// externally verified caller claim arrives as (ADR-0016 §3,
/// ADR-0024 §3); everything else (`LocalElevated`, the stage-4 `System`
/// context, and any future substrate-injected level) carries no externally
/// checkable DID and maps to `Internal`.
fn caller_binding(caller: &CallerContext) -> CallerBinding<'_> {
    match caller.auth {
        AuthLevel::Delegated | AuthLevel::Ucan => CallerBinding::Verified(&caller.caller_did),
        _ => CallerBinding::Internal,
    }
}
```

`convert::draft_in` is the parse floor: `serde_json::from_str::<Value>` on
`payload`, mapped to `invalid-record("payload is not valid JSON: …")`.

`caller_binding` is placed once in `host_capabilities.rs` and used by
`HostState`'s `signing::Host` impl only — the native-dispatch path (§8.3)
builds its own `CallerBinding` from `NativeInvocation.caller` directly,
since `SynSvcNativeService` does not go through `HostState`.

3. `crates/sandbox_wasm/src/engine.rs`:

- new field `pub record_signer: OnceLock<Arc<NodeRecordSigner>>` on
  `AppSandboxEngine`, defaulted in `init` — **not** a new `init`
  parameter (F7);
- `build_wasm_linker` gains one line after `conversation::add_to_linker`:

```rust
        syneroym_wit_interfaces::signing_host::syneroym::signing::signing::add_to_linker::<
            _,
            HasSelf<HostState>,
        >(&mut linker, |state| state)?;
```

- the `HostState::new(…)` chain at `engine.rs:1286` gains
  `.with_record_signer(self.record_signer.get().cloned())`.

`Cargo.toml`: `syneroym-signed-record` is reached through `syneroym-core`;
no new direct dependency.

### 8.2 `syneroym-app-host-native` — the shim

**`crates/app_host_native/src/factory.rs`**

- new field `record_signer: OnceLock<Arc<NodeRecordSigner>>` on
  `NativeHostFactory`, `OnceLock::new()` in `new` (F7 — 12 call sites, so
  a setter, not a parameter);
- new setter beside `set_service_proxy`:

```rust
    pub fn set_record_signer(&self, signer: Arc<NodeRecordSigner>) {
        let _ = self.record_signer.set(signer);
    }
```

- `build_host_state` (`:240`) gains
  `.with_record_signer(self.record_signer.get().cloned())` on the chain.

**`crates/app_host_native/src/host.rs`** — one new impl, after
`impl AppVault for NativeAppHost` (`:528`):

```rust
impl AppSigning for NativeAppHost {
    async fn sign_record(
        &self,
        draft: RecordDraft,
        as_principal: Principal,
    ) -> Result<String, SigningError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostSigning::sign_record(&mut *state, convert::draft_out(draft), convert::principal_out(as_principal))
            .await
            .map_err(convert::signing_error_guest)
    }

    async fn signing_identity(&self) -> Result<SigningIdentity, SigningError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostSigning::identity(&mut *state)
            .await
            .map(convert::signing_identity_guest)
            .map_err(convert::signing_error_guest)
    }
}
```

**`crates/app_host_native/src/convert.rs`** — four new converters in the
existing style, plus a round-trip test per converter matching
`vault_error_round_trips_all_variants` (`:716`):

| Converter | Direction |
|---|---|
| `draft_out(GuestRecordDraft) -> HostRecordDraft` | guest → host |
| `principal_out(GuestPrincipal) -> HostPrincipal` | guest → host |
| `signing_error_guest(HostSigningError) -> GuestSigningError` | host → guest |
| `signing_identity_guest(HostSigningIdentity) -> GuestSigningIdentity` | host → guest |

(The `wit_interfaces::signing` guest types and the
`wit_interfaces::signing_host` host types are structurally identical but
are two distinct Rust types, exactly as `vault`'s already are.)

### 8.3 `syneroym-control-plane` — the native-dispatch verb

**`ControlPlaneService` holds the one `NodeRecordSigner`; each
`SynSvcNativeService` gets it threaded on, at *both* construction sites —
missing either one is a live bug, not a style choice**, because
`orchestration.rs`'s own comment on the second site says so of every field
this constructor takes: "Mirrors `deploy_with_context`'s own construction
site in full -- every parameter, not an enumerated subset of it, or the
renewed service comes back with dead proxy/authorizer hooks." A service
that has just had its instance certificate renewed and silently lost
`signing` until its *next* full redeploy is exactly that failure, so this
plan names both sites rather than one.

**`crates/control_plane/src/service.rs`** — one new field on
`ControlPlaneService`, placed beside `conversation` (`:121`), with the
same reasoning that field's own doc comment already gives for choosing a
setter over a constructor parameter even though the value is available at
construction time — consistency with the fields threaded the same way
outweighs the one-line saving:

```rust
    /// The node's record signer, threaded on to each deployed service's
    /// `SynSvcNativeService` at construction time
    /// (`orchestration.rs`'s two `SynSvcNativeService::new` call sites).
    /// Strong `Arc`, not `Weak`: `NodeRecordSigner` holds no reference
    /// back to this service, so there is no cycle to guard against --
    /// unlike `service_proxy`/`row_authorizer` above.
    pub record_signer: OnceLock<Arc<syneroym_core::record_signer::NodeRecordSigner>>,
```

and a helper beside `current_conversation`:

```rust
    fn current_record_signer(&self) -> Option<Arc<syneroym_core::record_signer::NodeRecordSigner>> {
        self.record_signer.get().cloned()
    }
```

**`crates/control_plane/src/synsvc_native.rs`**

1. New field on `SynSvcNativeService`: `record_signer:
   std::sync::OnceLock<Arc<NodeRecordSigner>>`, mirroring `conversation`'s
   own field shape (`:142`, minus the `Weak` — no cycle here), plus a
   setter mirroring `set_conversation` (`:561`) exactly:

```rust
    pub fn set_record_signer(&self, signer: Arc<NodeRecordSigner>) {
        let _ = self.record_signer.set(signer);
    }

    fn current_record_signer(&self) -> Option<Arc<NodeRecordSigner>> {
        self.record_signer.get().cloned()
    }
```

2. **Both** `SynSvcNativeService::new(...)` call sites
   (`orchestration.rs:2155`, the `deploy` path, and `orchestration.rs:3129`
   inside `renew_cert_impl`) gain the identical extra line right after
   their existing `native_service.set_conversation(self.current_conversation());`:

```diff
             native_service.set_conversation(self.current_conversation());
+            native_service.set_record_signer_from(self.current_record_signer());
             native_dispatch.insert(service_id.clone(), native_service as Arc<dyn NativeService>);
```

   where `set_record_signer_from` is a small wrapper on
   `SynSvcNativeService` that no-ops when `self.current_record_signer()` is
   `None` (a substrate that never wired a signer, e.g. an isolated unit
   test), so the two call sites do not need to duplicate an `if let`:

```rust
    fn set_record_signer_from(&self, signer: Option<Arc<NodeRecordSigner>>) {
        if let Some(signer) = signer {
            self.set_record_signer(signer);
        }
    }
```

3. New `dispatch_signing`, placed after `dispatch_vault` (`:1305`):

```rust
    async fn dispatch_signing(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let Some(signer) = self.current_record_signer() else {
            return Err(internal("this node has no record signer configured"));
        };
        let caller = match invocation.caller.auth {
            AuthLevel::Delegated | AuthLevel::Ucan => {
                CallerBinding::Verified(&invocation.caller.caller_did)
            }
            _ => CallerBinding::Internal,
        };
        match invocation.method.as_str() {
            "sign-record" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    draft: RecordDraftDto,
                    #[serde(alias = "as-principal")]
                    as_principal: PrincipalDto,
                }
                let req: Req = parse_params(&invocation)?;
                let json = signer
                    .sign_record(
                        &self.service_id,
                        req.draft.into_draft()?,
                        &req.as_principal.into(),
                        caller,
                    )
                    .map_err(signing_error)?;
                to_payload(&Value::String(json))
            }
            "identity" => {
                let id = signer.identity(&self.service_id);
                to_payload(&serde_json::json!({
                    "signing-did": id.signing_did,
                    "pubkey-hex": id.pubkey_hex,
                    "owner-did": id.owner_did,
                }))
            }
            other => Err(RpcError::MethodNotFound(format!("signing/{other}"))),
        }
    }
```

with

```rust
fn signing_error(e: syneroym_core::record_signer::SigningError) -> RpcError {
    use syneroym_core::record_signer::SigningError as SE;
    match e {
        SE::PermissionDenied => {
            RpcError::Custom(PERMISSION_DENIED_CODE, "permission denied".to_string(), None)
        }
        SE::NoDelegation(msg) => RpcError::Custom(-32020, msg, None),
        SE::InvalidRecord(msg) => RpcError::InvalidParams(msg),
        SE::Internal(msg) => internal(msg),
    }
}
```

(`-32020` is unused today: `conversation_error` uses `-32001..-32003`,
`data_layer_error` `-32011..-32013`, `blob` its own block. Confirm with a
grep before committing to it, and add the same
`maps_every_variant_to_a_distinguishable_code` unit test
`data_layer_error` already has at `:1638`.)

4. `use syneroym_rpc::{ … }` gains `AuthLevel` (not imported by this file
   today — confirm with a grep before assuming it, since an unused-import
   warning on a workspace with deny-level clippy fails the gate).

5. One new arm in `NativeService::dispatch` (`:1616-1626`):

```diff
             "conversation" => self.dispatch_conversation(invocation).await,
+            "signing" => self.dispatch_signing(invocation).await,
```

6. Unit test: `both_synsvcnativeservice_new_call_sites_thread_the_record_signer`
   — construct a service via each of `deploy` and `renew_cert_impl` against
   a `ControlPlaneService` with a real `record_signer` wired, and assert
   `signing/identity` answers on the service produced by *each* path. A
   test that only exercises `deploy` would not have caught the original
   two-site gap. **Placement:** this test cannot live in `synsvc_native.rs`'s
   own `mod tests`, because `deploy_with_context`
   (`orchestration.rs:1391`) and `renew_cert_impl` (`orchestration.rs:3050`)
   are private methods of `orchestration.rs` — same crate is not enough for
   privacy. Place it in `orchestration.rs`'s own `mod tests` block instead,
   or assert it in `crates/substrate/tests/cert_renewal_e2e.rs`, which
   already drives the renewal path for real and can reach both code paths
   through the public API.

### 8.4 `syneroym-core` — `NATIVE_CAPABILITY_INTERFACES`

```diff
-pub const NATIVE_CAPABILITY_INTERFACES: [&str; 7] = [
+pub const NATIVE_CAPABILITY_INTERFACES: [&str; 8] = [
     "data-layer",
     "vault",
     "app-config",
     "blob-store",
     "messaging",
     HTTP_NATIVE_INTERFACE,
     "conversation",
+    "signing",
 ];
```

Consequences, all automatic (F8, F9): every deployed service gets a
`signing` `NativeHostChannel` at deploy
(`orchestration.rs:2123`); `list` keeps filtering it out (`:3269`);
`resolve_single_interface` keeps ignoring it (`local_registry.rs:212`);
`check_native_capability_gate` refuses a cross-service `signing` proxy call
and permits the same-service one (`router/src/proxy.rs:641`).
`crates/router/tests/native_dispatch_identity.rs:246` iterates one more
entry and needs no edit.

---

## §9 `syneroym-substrate` — wiring

### 9.1 `build_route_handler_deps` (`crates/substrate/src/runtime.rs:1907`)

`node_identity` is already in scope at `:1918`, and `registry` is a
parameter. After the `websocket_senders` wiring block (`:1953-1957`):

```rust
    let record_signer =
        syneroym_core::record_signer::NodeRecordSigner::new(node_identity.clone(), registry.clone());
    app_sandbox_engine
        .record_signer
        .set(record_signer.clone())
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::record_signer set more than once"))?;
```

`node_identity` is moved into `ControlPlaneService::init` at `:2058`, so
clone the `Arc` before that call (the file already does this for
`supervisor_client_identity` at `:1987` and explains why in a comment).

After `let control_plane_service = Arc::new(control_plane_service);`
(`:2061`), wire the native-dispatch side:

```rust
    control_plane_service.set_record_signer(record_signer.clone());
```

(`ControlPlaneService` forwards it to each `SynSvcNativeService` it builds
per deploy — thread it through the same way `conversation` is threaded at
`:2067`.)

`SharedNodeHandles` (`:876`) gains:

```rust
    /// The node's record signer, handed to every `NativeHostFactory` a
    /// linked app builds.
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    pub record_signer: Arc<syneroym_core::record_signer::NodeRecordSigner>,
```

set from `record_signer.clone()` in the `SharedNodeHandles { … }` literal
at `:2071`.

### 9.2 `init_roym` and `init_dual_build_fixture`

Each `NativeHostFactory::new(…)` call is followed by:

```rust
    factory_x.set_record_signer(shared.record_signer.clone());
```

Six call sites in `init_roym` (`runtime.rs:1547`, `:1577`, `:1608`,
`:1639`, and the two after it), one in `init_dual_build_fixture`
(`:1350`).

`RoymRole` (`crates/core/src/config.rs:417`) gains:

```rust
pub struct RoymRole {
    pub ui_bundle_path: Option<PathBuf>,
    /// The DID to record as the owner of the six natively linked services.
    /// A linked app has no deploy record, so nothing else records an owner
    /// -- and the owner is what the per-service signing key is derived
    /// from. Setting it makes the native build derive the same key a
    /// deployed build would, which is what lets one `record-signing`
    /// certificate work against either. Absent falls back to the node's own
    /// DID (see `NodeRecordSigner::service_identity`).
    pub owner_did: Option<String>,
}
```

and `init_roym`, before it builds the factories:

```rust
    if let Some(owner) = config.roles.roym.as_ref().and_then(|r| r.owner_did.clone()) {
        for svc in services::ALL {
            endpoint_registry.set_owner(roym_dispatch_id(svc.name), owner.clone()).await?;
        }
    }
```

---

## §10 The fixture — `test-components/dual-build-fixture`

`D-C3-14`: this is where the interface is proven, exactly as C1 proved its
four.

### 10.1 `wit/world.wit`

```diff
     import syneroym:vault/vault@0.1.0;
+    import syneroym:signing/signing@0.1.0;
     import syneroym:http/websocket@0.1.0;
```

### 10.2 New symlink

```
test-components/dual-build-fixture/wit/deps/signing/signing.wit
  -> ../../../../../crates/wit_interfaces/wit/signing/signing.wit
```

(F10 of the C1 plan: these are symlinks, not copies. The depth differs per
tree: the fixture's existing `wit/deps/proxy/proxy.wit` is
`../../../../../crates/wit_interfaces/wit/proxy/proxy.wit`, while
`crates/roym_profile/wit/deps/proxy/proxy.wit` is
`../../../../wit_interfaces/wit/proxy/proxy.wit`. Copy the sibling
symlink's target and swap `proxy` for `signing` rather than counting by
hand.)

`Cargo.toml`, `[package.metadata.component.target.dependencies]`:

```toml
"syneroym:signing" = { path = "wit/deps/signing" }
```

and `"signing"` added to its `syneroym-wit-interfaces` feature list.

### 10.3 `src/guest.rs`

One more entry in the `generate!` `with:` map:

```rust
            "syneroym:signing/signing@0.1.0":
                syneroym_wit_interfaces::signing::syneroym::signing::signing,
```

### 10.4 `src/app.rs` — four new verbs

```rust
    /// signing: build a draft, sign it as the service, return the envelope.
    SignAsService {
        record_type: String,
        version: u32,
        subject: String,
        payload: Value,
        expires_at_secs: Option<u64>,
    },
    /// signing: the same, under a delegation the test supplies.
    SignAsDelegated {
        record_type: String,
        version: u32,
        subject: String,
        payload: Value,
        delegation: String,
    },
    /// signing: what this service can sign as.
    SigningIdentity,
    /// The verification half, run inside the guest -- guest-side Rust so
    /// the consumer reaches its own verdict rather than depending on the
    /// node it is talking to. `revoked_dids`/`revoked_records` build a
    /// `RevocationSet` for the call; both empty is `EmptyRevocations`'
    /// coverage.
    VerifyRecord {
        envelope: String,
        expected_issuer: Option<String>,
        now_secs: u64,
        revoked_dids: Vec<String>,
        revoked_records: Vec<String>,
    },
```

Handlers return `Ok(json!({ … }))` on success and
`Ok(json!({ "error": fmt_err(e) }))` on refusal, matching
`RevealSecret`'s shape at `app.rs:618` — so a refusal is a comparable
value on both builds rather than a WIT `Err` the two builds map
differently. `VerifyRecord`'s success payload includes `revocation_status`
(`"good"` / `"unknown"`) alongside the rest of `VerifiedRecord`, so the
parity comparison covers it too.

`VerifyRecord` calls `syneroym_signed_record::verify_json`. The fixture's
`Cargo.toml` gains `syneroym-signed-record` as a target-independent
dependency (it is not a `roym_*` crate, so `check-roym-deps` does not look
at it).

### 10.5 The six `crates/roym_*` service crates — forced-import wiring, missed by a first pass

**This is not optional, and it is not covered by §10 or §11.** C2's own
plan found that `syneroym-app-host`'s `Cargo.toml` requests all eight
`wit_interfaces` guest features **unconditionally**, not target-gated
(`crates/app_host/Cargo.toml:10-27`), so Cargo feature unification forces
every crate that depends on `syneroym-app-host` — every one of `roym_web`,
`roym_conversation`, `roym_profile`, `roym_catalog`, `roym_transaction`,
`roym_directory` — to compile against `wit_interfaces` with all eight
features regardless of what that crate's own `Cargo.toml` says. §6.1 adds
`signing` to that list, which makes it the **ninth** forced import. If the
six service crates are left as they are, `cargo component build -p
syneroym-roym-profile --target wasm32-wasip2` fails: `wit_interfaces`
compiles `syneroym:signing`'s guest bindings into the build graph, but
`roym_profile`'s own `wit/world.wit` does not import `syneroym:signing`,
so the world the component encodes against does not match what the linked
bindings expect.

For **each** of the six crates, apply the same five edits C2's own §3.3/
§3.4 pattern already establishes for the other eight interfaces:

1. `wit/world.wit` — one new import line, alongside the existing eight
   (order matches the existing list; put it after `vault`, before `http`,
   matching §10.1's fixture edit):

```diff
     import syneroym:vault/vault@0.1.0;
+    import syneroym:signing/signing@0.1.0;
     import syneroym:http/websocket@0.1.0;
```

2. `wit/deps/signing/signing.wit` — a symlink, not a copy, matching the
   crate's *existing* sibling depth exactly (verified for `roym_profile`:
   `wit/deps/proxy/proxy.wit` → `../../../../wit_interfaces/wit/proxy/proxy.wit`,
   four levels — copy that target and swap `proxy` for `signing` rather
   than counting by hand, since a wrong depth resolves silently to nothing
   the build tool complains about until link time).

3. `Cargo.toml`, `[target.'cfg(target_arch = "wasm32")'.dependencies]` —
   add `"signing"` to the `syneroym-wit-interfaces` feature list (nine
   entries now, keep alphabetical).

4. `Cargo.toml`, `[package.metadata.component.target.dependencies]` — add:

```toml
"syneroym:signing" = { path = "wit/deps/signing" }
```

5. `src/guest.rs`'s `generate!` `with:` map — one more remap entry,
   identical to the fixture's (§10.3):

```rust
            "syneroym:signing/signing@0.1.0":
                syneroym_wit_interfaces::signing::syneroym::signing::signing,
```

None of the six crates' `world.wit` needs to **export** anything new —
`signing-import` has no export requirement, the same reason `proxy`,
`vault`, and `app-config` cost nothing on the export side either (C2's own
§3.3 finding, unchanged here).

**Backlog consequence.** `deferred-backlog.md` already carries a row for
the forced-import cost, costed at "all eight interfaces" (added by C2).
§16 below updates it to nine and points at this section rather than
silently leaving the count wrong.

---

## §11 `syneroym-roym-core` — the dependency and the re-export

`D-C3-14`: no verbs, no record types, no product behaviour. Just enough
that C4 starts from a compiling base and that the envelope has one name in
the product.

### 11.1 `crates/roym_core/Cargo.toml`

```toml
[dependencies]
syneroym-app-host.workspace = true
syneroym-signed-record.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true

[dev-dependencies]
toml.workspace = true
syneroym-identity.workspace = true    # test-only: generating a keypair is
                                      # the only way to test verification
```

### 11.2 `crates/roym_core/src/record.rs` — new file

```rust
//! The signed record envelope, as the product sees it, plus the record
//! types the spec fixes. Signing itself is a host call
//! (`AppSigning::sign_record`); verification is this code, running
//! identically in both builds -- a consumer's node reaches its own verdict
//! and never takes a directory's word for one.

pub use syneroym_signed_record::{
    verify, verify_json, EmptyRevocations, Envelope, RecordDraft, RevocationSet, RevocationSource,
    VerifiedRecord, VerifyError, VerifyOptions,
};

/// The nine record types, and the version each is produced at today.
/// Mirrors `card::CARD_TYPES`: fixed, and a record of an unlisted type or
/// an unlisted version is not understood rather than guessed.
pub const RECORD_TYPES: &[(&str, u32)] = &[
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

pub fn is_known_record(record_type: &str, version: u32) -> bool {
    RECORD_TYPES.iter().any(|&(t, v)| t == record_type && v == version)
}
```

`crates/roym_core/src/lib.rs` gains `pub mod record;`.

Tests: every `RECORD_TYPES` entry passes `RecordDraft::validate`'s
`record_type` shape rule; a locally signed envelope round-trips through
`verify`; a tampered one does not.

### 11.3 `xtask/src/main.rs:70`

```diff
     let allowed_target_independent = [
         "syneroym-app-host",
         "syneroym-roym-core",
+        "syneroym-signed-record",
         "serde",
         "serde_json",
         "async-trait",
         "thiserror",
     ];
```

with a comment stating why this one is allowed and `syneroym-identity` is
not: `syneroym-signed-record` is a pure format-and-verification crate,
compiled identically into both builds, and it exposes no entry point that
can produce a signature. D3 forbids the native build reaching a host
capability the WASM build cannot; it does not forbid both builds sharing
one definition of a byte format.

---

## §12 `roymctl` and the SDK — minting the delegation

### 12.1 `crates/sdk/src/deploy.rs`

```rust
/// Mint a `record-signing` delegation from `master` over the key
/// `substrate` derives for `service_id` **under this client's own
/// identity**. The sibling of [`certify_instance`], and deliberately not a
/// variant of it: that one requires `master_did == service_id` (an
/// ADR-0020 member master) and mints `service-instance` scope. This one
/// names any master DID and mints `record-signing`, which no transport
/// path accepts -- `master` may be a natural person's own identity or an
/// organisation's own dedicated one (a SynOrg's own minted master, say);
/// nothing here or on the verifying side distinguishes the two.
///
/// The querying identity matters: the substrate derives the key from its
/// own node identity **and** the calling DID, and the record signer derives
/// it from the service's recorded owner -- so a certificate minted through
/// a client that is not the recorded owner is refused at signing time, not
/// here.
pub async fn certify_record_signing(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let identity = client.instance_identity(service_id).await
        .context("failed to query the substrate for its derived signing key")?;
    let pubkey = pubkey_from_hex(&identity.pubkey_hex)?;   // extracted from
                                                           // certificate_over_instance_identity
    DelegationCertificate::issue(
        master,
        pubkey,
        expires_hours * 3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
}
```

Extract the three-step hex→`VerifyingKey` decode at
`crates/sdk/src/deploy.rs:662-668` into `fn pubkey_from_hex` and call it
from both places, rather than copying it.

### 12.2 `apps/roymctl/src/commands/identity.rs`

New `IdentityCommands` variant, after `CertifyInstance`:

```rust
    /// Certify the record-signing key a substrate derives for a service, so
    /// that service can sign product records under *this identity's* DID --
    /// a natural person's own master, or a dedicated master an organisation
    /// mints for itself and holds separately from any individual's personal
    /// identity.
    ///
    /// Distinct from `certify-instance`, which certifies the same key for
    /// ADR-0020 *transport* identity under a member master. This one is
    /// scoped `record-signing` and is refused on every transport path.
    ///
    /// Queries `--substrate` over `orchestrator/resolve-instance-identity`
    /// (gated on `orchestrator/status`), so pass `--as <name>` naming the
    /// same identity the service was deployed under -- the substrate
    /// derives the key from the caller, and the record signer derives it
    /// from the recorded owner. A certificate minted as anyone else is
    /// accepted here and refused at signing time.
    CertifySigning {
        /// Name of the local identity issuing the certificate -- a
        /// person's own master, or an organisation's dedicated one.
        #[arg(long)]
        master: String,
        /// The service whose signing key is being certified.
        #[arg(long)]
        service_id: String,
        /// DID of the substrate to query.
        #[arg(long)]
        substrate: String,
        #[arg(long, default_value_t = 24)]
        expires_hours: u64,
    },
```

Handler mirrors `CertifyInstance` (`identity.rs:237-256`) minus the anchor
refresh (`record-signing` never travels on the wire, so no master anchor is
needed) and prints the certificate JSON.

Tests in `apps/roymctl`: the command parses; the minted certificate has
scope `record-signing`; and it is **rejected** by
`DelegationCertificate::verify(master, &TRANSPORT_SCOPES)` — the negative
test `D-C3-5` exists for.

### 12.3 What this section still does not solve

`certify-signing` needs the issuing identity's master key file **on the
local machine running `roymctl`**. It has no answer for the Hub's own
browser-only person (§16's certificate-lifecycle backlog row), and it
mints one certificate for one service — Roym has six, so a person signing
records across more than one of them needs to run this command six times
and track six separate expiries by hand. Neither gap is closed here; both
are named once, in §16, rather than left to be rediscovered by C4.

---

## §13 Tests

### 13.1 What each suite is for

| Suite | Proves |
|---|---|
| `crates/signed_record/src/**` unit tests | Every refusal rule and every verification failure mode, with no substrate in the picture (§3.5). |
| `crates/core/src/record_signer.rs` unit tests | The derivation matches the other two call sites; the delegated-certificate checks hold, including the delegation-window regression and the caller-binding check; two signers with the same inputs produce identical bytes (§7.4). |
| `crates/app_host_native/src/convert.rs` unit tests | Every new converter round-trips every variant. |
| `crates/control_plane/src/synsvc_native.rs` unit test | Every `SigningError` maps to a distinguishable RPC code. |
| `crates/app_host_native/tests/dual_build_parity.rs` | The interface behaves identically on both builds (§13.2). |
| `crates/router/src/proxy.rs` unit tests | A guest reaching *another* service's `signing` is refused, and its own is allowed. The two existing gate tests (`:3119`, `:3273`) hardcode `"data-layer"`, so adding `"signing"` to the array exercises the mechanism but names nothing — add a `signing`-specific pair beside them. |
| `crates/substrate/tests/record_signing_e2e.rs` (new) | The one thing no unit or parity suite reaches: a service deployed through the **real** `ControlPlaneService::deploy` gets a `signing` native-capability endpoint and a recorded `owner_of`, and a certificate minted the way `roymctl identity certify-signing` mints one is accepted by that service's signer (§13.3). This is the actual product path, and it is where `resolve-instance-identity`'s caller-derivation and the signer's owner-derivation are proven to agree against a live substrate rather than by construction. |
| `crates/roym_core` unit tests | The nine record-type names are valid `record_type`s, and a signed envelope round-trips through `verify`. |
| `apps/roymctl` unit tests | `certify-signing` mints `record-signing` scope, and that scope is refused on transport. |

### 13.2 New parity scenarios

Added to `crates/app_host_native/tests/dual_build_parity.rs`. Both stacks
are given a `NodeRecordSigner` built from the **same** `Arc<Identity>`, the
same `owner_did` recorded for the fixture's service id, and
`RecordClock::Fixed(1_800_000_000)`.

> **Clock-compatibility constraint.** `DelegationCertificate::issue` stamps
> `issued_at = now` (real wall clock) and `expires_at = now + expires_in`
> with no way to set the start. The verifier rule for a delegated envelope
> requires `cert.issued_at ≤ record.issued_at < cert.expires_at`, and the
> signer separately checks the certificate is valid at the real wall clock.
> `RecordClock::Fixed(1_800_000_000)` is 2027-01-15 — in the future — so a
> certificate minted today (2026-08-31) must stay valid until at least that
> timestamp. Mint all parity certificates with `expires_hours` large enough
> to straddle both clocks: `ceil((1_800_000_000 - now_secs) / 3600) + 24`
> hours gives a safe margin. As calendar time advances toward that constant,
> re-evaluate and either raise `RecordClock::Fixed` to stay ahead of the
> wall clock, or switch to a constant offset from `now_secs` — byte-identity
> only requires a constant, not a future one. Scenarios 3, 6, 7, 8, and 17
> are the ones that depend on this.

| # | Scenario | Assertion |
|---|---|---|
| 1 | `SigningIdentity` | Same `signing_did`, `pubkey_hex`, `owner_did` on both builds. |
| 2 | `SignAsService` on a well-formed draft | The two envelopes are **byte-identical** (F13), both verify, and `issuer == signing_did`. |
| 3 | `SignAsDelegated` with a valid certificate, driven by a caller whose verified DID matches the certificate's master | Byte-identical, both verify, `issuer` is the certificate's master DID, `delegation` is present. |
| 4 | `SignAsDelegated` with a certificate over a different key | Same `no-delegation` refusal text on both builds. |
| 5 | `SignAsDelegated` with a `service-instance`-scoped certificate | Same `no-delegation` refusal on both. |
| 6 | `SignAsDelegated` with a certificate that has since wall-clock expired | Same `no-delegation` refusal on both — the *signer* refuses signing under an already-expired delegation, distinct from scenario 17 below, which is about a *record already signed* while the delegation was still good. |
| 7 | `SignAsDelegated` with a valid certificate, driven by a caller whose verified DID does **not** match the certificate's master | Same `no-delegation` refusal on both, naming the mismatch — the regression test for the caller-binding check the review pass added (`D-C3-4`). |
| 8 | Scenario 3, repeated with the driving caller built as `CallerBinding::Internal` (a cross-service sibling call via `service_system` rather than a verified session) | Succeeds identically on both builds — the caller-binding check does not apply when there is no externally verified DID to check against, proven rather than assumed. |
| 9 | `SignAsService` with `version: 0` | Same `invalid-record` refusal on both. |
| 10 | `SignAsService` with a float in the payload | Same `invalid-record` refusal on both, naming the field. |
| 11 | `SignAsService` with a payload that is a JSON array | Same `invalid-record` refusal on both. |
| 12 | `SignAsService` with a 65 KiB payload | Same `invalid-record` refusal on both. |
| 13 | `VerifyRecord` on a valid envelope with the right `expected_issuer` | Same `VerifiedRecord` on both, including the derived `record_id` and `revocation_status == Good`. |
| 14 | `VerifyRecord` with the wrong `expected_issuer` | Same `IssuerMismatch` on both. |
| 15 | `VerifyRecord` on an envelope with one payload byte flipped | Same `BadSignature` on both. |
| 16 | `VerifyRecord` on an envelope past `expires_at_secs` | Same `Expired` on both. |
| 17 | `VerifyRecord`, with `now_secs` set **past the signing certificate's own `expires_at_secs`**, on a `SignAsDelegated` envelope that was signed while that certificate was still valid | Succeeds identically on both, with `revocation_status == Unknown` (no revocation source was supplied). This is the regression test for the delegation-window bug the review pass found: the original design used `cert.verify` (wall-clock) on the verifier side, which would fail this exact case. |
| 18 | `VerifyRecord` on an envelope with `version` deleted | Same parse failure on both. |
| 19 | `VerifyRecord` with `revoked_dids` naming the envelope's `issuer` | Same `RevokedKey` on both. |
| 20 | `VerifyRecord` with `revoked_records` naming the envelope's own derived `record_id` | Same `RevokedRecord` on both. |
| 21 | `SignAsService` twice with the same draft and the same fixed clock | Identical bytes within each build too — the record is a pure function of its inputs, which is what `record_id` being content-derived rests on (`D-C3-9`'s idempotence note). |

### 13.3 `crates/substrate/tests/record_signing_e2e.rs`

One substrate, one deployed service, one person identity. **No WASM
component and no `dual_build_fixture` feature**: `signing` is a native
capability every deployed service gets regardless of type (F8), and F15
makes it reachable by an authenticated client — so the test deploys the
same minimal service `instance_identity_e2e.rs` already deploys
(`bare_tcp_manifest`) and drives `signing` over native dispatch. That file
is also the model for the deploy-then-certify dance; copy its harness
rather than writing a new one.

```
1. Boot a substrate; claim it with `person` as controller.
2. Deploy a bare service as `person`
     -> registry records owner_of = person, and a `signing`
        NativeHostChannel endpoint (F8).
3. Call `signing/identity` on it as `person`
     -> assert: owner_did == person's DID, and pubkey_hex equals what
        `orchestrator/resolve-instance-identity` returns for the same
        caller. This is the three-call-site derivation (§7.2) proven
        against a live substrate rather than by construction.
4. Call `signing/sign-record` with `as-principal = service`
     -> assert: `verify` succeeds with expected_issuer = signing_did.
5. Mint a `record-signing` certificate over that key with `person`'s
   master (`syneroym_sdk::deploy::certify_record_signing`) and call
   `signing/sign-record` as `person`, with `as-principal = delegated(cert)`
     -> assert: issuer is `person`'s DID; `verify` with
        expected_issuer = person succeeds.
6. Mint a certificate as a *second* identity, `stranger`, that never
   deployed anything, and repeat step 5 as `stranger`
     -> assert: refused, naming the key mismatch (`stranger`'s own
        derived key is not this service's signing key, since
        `resolve-instance-identity` derives under the *caller's* DID).
        This is the constraint "the identity who can certify is the
        deployer" proven rather than asserted in prose.
7. Assert `person`'s certificate from step 5 is refused on the transport
   path: `cert.verify(master, &TRANSPORT_SCOPES)` is `Err`.
8. Call `signing/sign-record` on that service **as `stranger`**, with
   `as-principal = service`
     -> assert: it succeeds, and record that this is the accepted posture
        for `principal::service` -- any caller the router admits reaches
        it, unbounded. Not an oversight: a named, tested residual with its
        own backlog row. When a later slice adds a capability gate, this
        assertion inverts and that row closes.
9. Call `signing/sign-record` on that service **as `stranger`**, with
   `as-principal = delegated(cert)` where `cert` is `person`'s valid
   certificate from step 5 (obtained by `stranger` out of band, e.g.
   copy-pasted)
     -> assert: refused, naming the mismatch between `stranger`'s own
        verified session DID and the certificate's master DID `person`.
        This is the caller-binding check the review pass added, proven
        against a live substrate with two real, distinct authenticated
        identities rather than only in the crate-local unit tests -- the
        one case §7.4's unit test cannot fully exercise, since it builds
        `CallerBinding` by hand rather than through a real router-verified
        session.
```

### 13.4 Failure-and-security-matrix rows C3 closes

Rows **1** (mechanism only — the rendering half is C6/C7), **10** (a
correction is a new record carrying `supersedes`, and neither can be
altered), and **14** (a record with no version fails to parse). Row **19**
(any interface behaving differently on the two builds) gains 16 cases.
Record every one of these in `status.md`.

---

## §14 Order of work

Each step compiles and its own tests pass before the next begins.

1. `syneroym-identity`: `SCOPE_RECORD_SIGNING`, the `TRANSPORT_SCOPES`
   negative test, the three `wasm32` warnings (§4).
2. `crates/signed_record`: the whole crate and its unit tests (§3),
   **including** the delegation-window regression tests and the
   revocation tri-state tests. Verify
   `cargo build -p syneroym-signed-record --target wasm32-wasip2`.
3. `syneroym-core`: `NodeRecordSigner`, `CallerBinding`, and their unit
   tests (§7).
4. The WIT package and both bindgen modules (§5). Verify both `generate!`
   passes compile before anything depends on them.
5. `syneroym-app-host`: `AppSigning`, the `AppHost` bound, the guest
   bridge (§6). The workspace will not compile again until step 6 lands.
6. `syneroym-sandbox-wasm` and `syneroym-app-host-native` (§8.1, §8.2) —
   the two `AppHost` implementors, including `HostState`'s
   `caller_binding` helper. Workspace compiles again here.
7. `NATIVE_CAPABILITY_INTERFACES`, `ControlPlaneService.record_signer`,
   and the native-dispatch verb at **both** `SynSvcNativeService::new`
   call sites (§8.3, §8.4). The
   `both_synsvcnativeservice_new_call_sites_thread_the_record_signer`
   test (§8.3.6) is the check this step must not skip.
8. `syneroym-substrate` wiring, `RoymRole::owner_did` (§9).
9. The fixture: WIT, symlink, guest remap, four verbs (§10). Rebuild with
   `mise run build:test-components`.
10. **The six `crates/roym_*` service crates' WIT plumbing (§10.5)** —
    all five edits, all six crates. Skipping this step is invisible until
    step 11 tries to build the WASM components and fails with a
    world/bindings mismatch, not before.
11. The parity scenarios (§13.2), the router gate pair (§13.1), and
    `crates/substrate/tests/record_signing_e2e.rs` (§13.3).
12. `roym_core` + the xtask allowlist (§11). Rebuild with
    `mise run build:roym`; run `cargo xtask check-roym-deps`.
13. The SDK helper and the `roymctl` verb (§12).
14. The full gate: `cargo +nightly fmt --all`,
    `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace`, `cargo audit`,
    `cargo deny check licenses`, `mise run test:e2e`.
15. Docs and backlog (§16).

Steps 1–3 and step 4 are independent and can run in parallel. Step 5 is
the choke point: it breaks the build until step 6.

---

## §15 Permitted differences (WASM vs native)

To be appended to `status.md`'s existing §14 list.

1. **Nothing new.** Both builds reach the same `NodeRecordSigner` through
   the same `HostState`, so `signing` introduces no permitted difference —
   which is the point of routing the native shim through `HostState` in the
   first place (F6). If a divergence appears, it is a bug in the shim, not
   a difference to document (`task.md` failure-matrix row 19).
2. One inherited difference is worth restating because it now has teeth: a
   natively linked app has **no deploy record**, so
   `EndpointRegistry::owner_of` is `None` unless
   `[roles.roym] owner_did` is set, and the derived signing key is
   therefore different from the deployed build's. `RoymRole::owner_did`
   (§9.2) exists so a native deployment states its owner rather than
   drifting. The parity suite sets the owner on both stacks explicitly, so
   the suite itself never depends on the fallback.

---

## §16 Documents and backlog owed

Per `task.md`'s "Owed as slices land" table, **C3 completes** owes:

| Document | Edit |
|---|---|
| [status.md](status.md) | A C3 section: what shipped, the §13.4 matrix rows closed, and the §15 "no new permitted difference" statement. |
| [task.md](task.md) | **Gap 1 recorded as closed**, naming `syneroym:signing` and `syneroym-signed-record`, and naming what changed against the gap's own text: the person delegation is a *parameter*, not substrate state (`D-C3-4`), and the ADR-0020 instance certificate is not the person (F4). |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | The architecture section enumerates the WIT interface list (`host`, `data-layer`, `blob-store`, `app-config`, `control-plane`, `vault`) — add `signing`, and add `syneroym-signed-record` to the crate list. Also correct the same paragraph's already-stale omission of `http`, `messaging`, `conversation` and `proxy` while there. |
| [deferred-backlog.md](../../deferred-backlog.md) | **New row:** `principal::service` records carry no capability gate — any caller the router admits to a service can make it sign under its own identity, unbounded. The `D-C3-4` caller-binding check narrows `principal::delegated` but does not touch this case. Targeted at **C4**, sources `crates/sandbox_wasm/src/host_capabilities.rs` and `crates/control_plane/src/synsvc_native.rs`; a fix that covers `signing` alone while `vault/reveal` keeps the identical exposure is not a fix. **New row:** `principal::delegated` signing from an *internal* (substrate-injected) caller is not checked against any DID, because there is none to check against — proven, not assumed, by §13.2 scenario 8. C4 is the first slice that signs a person record through a sibling call (every such guest cross-service call arrives as `CallerBinding::Internal`), so this residual is load-bearing from the moment C4 starts — whoever builds C4 must decide whether an internal caller may drive `delegated` signing at all, or must be handed a verified identity some other way. Target **C4**. **New row: certificate lifecycle has no home.** One `roymctl identity certify-signing` invocation mints one certificate for one service, needs the issuing identity's master key file on the local machine, has no storage, no renewal path, and no route for a browser-only Hub person to obtain one at all (`D-C3-13`, §12.3). Roym has six services, so this is six certificates per person today. **Named as a hard prerequisite of C4**, not an incidental gap — C4 cannot ship a single record-producing UI flow without an answer. **New row:** the real `RevocationSource` (fetching and caching actual revocation lists / credential state) does not exist yet — C3 ships the tri-state shape (`D-C3-16`) and `RevocationSet` for tests, but nothing populates one from network state. Target **C9**, which is where the spec's revocation and credential rows land. **New row:** `canonicalize_json_value` is not RFC 8785 (F3) and the non-integer-number refusal is what covers the gap, trigger "a non-Rust producer or verifier of Roym records exists", target TBD. **New row:** a natively linked service's signing key changes if it later gains a recorded owner (§7.2), trigger "a node runs both the linked and the deployed build of the same app", target TBD. **New row (C2's, corrected):** the forced-import cost C2 costed at "all eight interfaces" is now **nine** — `syneroym:signing` joins the list every Roym service crate's `world.wit` must import regardless of use (§10.5). Update the existing row's count rather than leaving it stale. |
| [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md) | Its Consequences table's "Slice C3" row says the signing key is "the instance/service key **or a per-person delegation the substrate holds**". Amend: the substrate holds the *key*; the *certificate* is a per-call parameter (`D-C3-4`), checked against the calling session's own verified DID where one exists, and the reason it is a parameter rather than substrate state is F4 — no such per-person delegation exists in substrate state today. |
| C2's [slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) | Its §3.3 "all eight interfaces" language is now stale everywhere it counts imports rather than naming them explicitly — nine, after this slice. Not required for C3's own gate, but worth a pass so a reader of C2's plan is not misled about a count that changed after it was written. |

**No new ADR.** The signing interface is one WIT package and one crate,
decided inside a slice plan the way `D-06C-3` decided the card set. There
is no cross-slice negotiation and no wire format anyone outside Roym
consumes.

---

## §17 What "done" means for C3

1. `syneroym:signing@0.1.0` exists, is linked into the WASM linker, is
   reachable through `AppHost` on both builds, and is **not** in
   `host-environment` — a component that does not import it deploys
   unchanged.
2. `sign-record` refuses every rule in `D-C3-11` with a distinguishable
   error, and no function anywhere in the interface returns key material.
3. `principal::delegated` signs under any master DID a `record-signing`
   certificate names, and refuses a certificate over any other key, in any
   other scope, outside its own validity window, or -- for a verified
   caller -- naming a master that caller is not. A record signed while a
   delegation was valid still verifies after that delegation has since
   wall-clock expired; a record dated outside the delegation's window does
   not.
4. `syneroym-signed-record` compiles for the host and for
   `wasm32-wasip2`, exposes no `Identity`-taking entry point, and its
   verifier checks signature, issuer, delegation scope-and-window (against
   the record's own timestamp, not the wall clock), and revocation
   (tri-state: `Good`/`Revoked`/`Unknown`, never a bare pass) in the
   documented order.
5. All 21 parity scenarios pass identically on both builds, including the
   byte-identity assertions, the delegation-window regression, and the
   caller-binding check.
6. `cargo xtask check-roym-deps` is clean with `roym_core` depending on
   `syneroym-signed-record` and on nothing else new.
7. Grep proves no planning identifier (`C3`, `M06C`, `R1`, …) appears in
   any crate, module, interface, method, record type, config key, metric,
   or test name (`D-06C-11`).
8. The full gate in §14 step 14 is clean.
9. §16's documents and backlog rows are written.

---

## §18 Ambiguities and staleness in the input documents

Flagged rather than guessed, as asked. **(A)** and **(B)** change what gets
built; the rest are corrections owed to the documents.

**A. "Signs under the person's delegated key" has no artifact behind it,
and the two documents that say it name different things.**
[task.md](task.md)'s C3 row says "the signing key is the instance/service
key or a per-person delegation the substrate holds", and
[ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md)'s
Consequences table says the same. **The substrate holds no per-person
delegation.** The only certificate in registry state is ADR-0020's instance
certificate, and `verify_installed_instance_cert` requires its
`master_did` to equal the `service_id` — a member master, never a person
(F4). `owner_of` records the deploying person but is an unsigned local
string.

Resolved as `D-C3-3` / `D-C3-4`: C3 introduces the person delegation, and
it is a **per-call parameter** rather than substrate state, because the
certificate carries no private key and the host can bind it to the exact
key it signs with. This is a real deviation from both documents' wording
and is written back into ADR-0024 in §16. **If a reviewer wants the
delegation held in substrate state instead**, the cost is a new
`EndpointStorage` method triple, a new SQLite table, three
`EndpointRegistry` methods, a new orchestrator install verb with its own
authorization gate, and a boot-time replay path — and it would still not
support more than one person per service. Say so before choosing it.

**B. `task.md`'s Migration-impact section says "New traits on `AppHost`
for … outbound `websocket`" and lists the C1 set; C3 adds one more.**
`AppHost`'s supertrait list grows again, from eight to nine. Two
implementors, both in-tree, both listed in §6.3 — the same bounded cost
`task.md` names for C1. Not a problem, but the section reads as if C1 was
the last time it happens, and it is not.

**C. `task.md`'s Gap 1 says `Identity::sign`/`sign_json`/
`derive_service_identity` are at `keys.rs:195-240`.** Correct as of that
date and still correct: `sign` is at `:195`, `sign_json` at `:201`,
`derive_service_identity` at `:240`. No edit needed; recorded because the
line references in that section are dated and the next slice should
re-check rather than trust them.

**D. `task.md`'s "Open design points" asks C3 to pick among "the person's
master DID, the service's derived instance key, and the node".** The
**node** principal is excluded and not built: no record in the spec's table
is signed by a node, and `RelationshipProof` already covers the one
node-adjacent assertion the tree makes; that exclusion keeps a backlog row.
A fourth candidate the list does not name, the **member master** (the
principal the existing instance certificate actually expresses), was
excluded in a first pass and is **not excluded in the shipped design**: a
review pass found the exclusion did not survive `membership-credential`/
`revocation`/`moderation-decision` needing a stable SynOrg issuer, so
`D-C3-3` folds it into `principal::delegated` by construction rather than
adding a third variant — see that row for the reasoning.

**E. The spec's `revocation` record and key revocation are two different
things, and neither document separates them.** `MasterAnchorPayload.revoked_keys`
(`crates/core/src/dht_registry.rs:551`) revokes an instance *key*; the
spec's `revocation` record revokes a *credential*. C3's
`RevocationSource` has one method for each and fetches neither — a
verifier that asks a directory whether a record is good has handed the
directory the verdict, which failure-matrix row 2 forbids. C9 supplies the
real source for both. Named here because "verification of … revocation" in
the C3 row reads as one thing.

**F. `D-06C-1` says "every signed record carries an explicit version
field", singular.** C3 ships two (`D-C3-10`): `envelope_version` for the
container and `version` for the body. The decision's *reason* — adding a
field later changes the canonical bytes and invalidates every existing
signature — applies to the container at least as strongly as to the body,
so one field would leave the container unversioned. Flagged because it
reads like a departure and is not.

**G. `crates/roym_core/app/roym.toml`'s `custom_config` comment says
"fine-grained session auth gating is target for C4".** Still true, and C3
does not change it. Restated so a reader of that file does not expect C3 to
have closed it: C3 adds no route, no gate, and no caller check.

**H. The `CLAUDE.md`/`AGENTS.md` architecture paragraph's WIT list is
already stale before C3 touches it.** It names `host`, `data-layer`,
`blob-store`, `app-config`, `control-plane`, `vault` — omitting `http`
(M06A), `messaging`, `conversation` (M06B) and `proxy` (C1). §16 fixes all
of it in one pass rather than adding `signing` to a list that is already
wrong.

**I. `status.md`'s C2 section records that a sibling never learns who is
asking, and C3 does not change that.** A record's *issuer* is the service's
own signing key, or the master DID a `delegated` certificate names — never
"the caller". `D-C3-4`'s caller-binding check narrows a *different* gap (a
verified caller presenting someone else's certificate); it does nothing
for a sibling that never sees the caller at all. Nothing in C3 lets
`catalog` learn that the person driving `web` is the one asking. That gap
stays open, still targeted at M6's cross-service caller-identity spec
pass, and C4 inherits it. Worth stating because "signs under the person's
delegated key" can be misread as closing it.

**J. A review pass of this plan found the verifier side checking the
wrong clock against a person's delegation certificate.** The first draft
of §3.4 step 5 called `cert.verify` (wall-clock expiry) when checking a
`delegated` record's certificate at *verification* time. Since a
`record-signing` certificate is short-lived by design (`roymctl identity
certify-signing`'s own default is 24 hours) and a signed record is meant
to outlive it indefinitely, every `delegated` record would have stopped
verifying roughly a day after it was signed — silently failing R1's
listing round-trip row, failure-matrix row 13 ("a record that verified
before verifies after"), and exit criterion 7's export/import row. Fixed
by checking `cert.verify_chain` (no wall-clock expiry) plus an explicit
window check against the *record's own* `issued_at_secs`, which is the
question that actually matters: was the delegation valid when it was
used, not is it valid now. §3.4, §3.5, and §13.2 scenario 17 all carry
this fix and its regression test. Recorded here because it is exactly the
kind of defect a plan of this shape is supposed to catch before code is
written, and did not on the first pass.
