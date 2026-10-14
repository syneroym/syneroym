# M06D slice B1 — Passport core: implementation plan

*Reader: the developer (or AI) who implements slice B1. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "B1", "B2",
  "X4", "P-7", "H-1", "D-B1-3" in code, comments or test names; ADR ids are
  fine), simple English in comments and docs, the 800-line file cap, the
  100-line function cap, match arms of at most ~10 lines, and the module
  layout rule (`foo.rs` beside `foo/`, never `mod.rs` under `src/`).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 row X4, §2 items 1 to 3 and 6, §3 slices B1, B2, B3 and C8 (B2, B3 and
  C8 are the later users of this code — do not build them).
- `docs/planning/changes/operator-passport/change.md`: all of "Core", and
  "Use: home relay" for the relay facts.
- `docs/planning/changes/service-mailbox/change.md`: "Dedicated mailbox and
  passport" (the second user of this code).
- `docs/decisions/0015-ucan-capability-model.md`: §1, §2, A1, A3, A5 and
  **A10** (the passport amendment).
- §4 of this plan (open points). **Items marked "owner check" must be
  confirmed by the owner before you write the code they affect.**

**Branch.** The order is strictly serial (one slice per PR, each merged
before the next branches). Work on `feat/m06d-b1-implementation`. It was
branched from `main` at `86511e58`, after C1 (#305) merged. This plan is the
first commit on that branch.

### Terms used in this plan

- **Passport**: a `CapabilityToken` signed by the operator root. Its audience
  is the owner DID of the user. It has no proofs.
- **Child token**: a `CapabilityToken` signed by the owner. Its audience is the
  **subject**. Its one proof is the passport.
- **Subject**: the audience of the child token. For the relay it is the node
  DID of the user's substrate. For the mailbox it is the member master DID.
- **Proven key**: the DID whose private key the caller has shown it holds,
  either by a challenge signature or by the transport.
- **Signer binding**: the check "may the proven key act for the subject?".
- **Operator root**: a DID the service trusts to issue passports (the
  controller of the host substrate, or the host node DID).
- **Host node DID**: the node DID of the substrate that runs the service. It
  is part of the resource name and of the challenge signature.
- **Lease**: one row `(passport_id, owner_did, subject, lease_ends_at)`. A
  subject with a running lease is admitted.
- **Use**: one of the two users of the core: home relay (`relay/home`) or
  dedicated mailbox (`mailbox/use`).

## 1. What this slice delivers

A new crate `crates/passport` (`syneroym-passport`) with everything both uses
share, plus the operator's `issue` command. No service uses the crate yet. B2
wires the relay use into the coordinator; C8 wires the mailbox use.

1. `crates/ucan`: two new abilities (`relay/home`, `mailbox/use`), the
   passport id (`CapabilityToken::token_id`), and a public `sign` method so a
   token with an exact expiry can be built.
2. `crates/passport`:
   - `PassportUse` (the per-use settings) with the two constants
     `RELAY_HOME` and `MAILBOX_USE`.
   - `issue_passport` and `child_token`.
   - `PassportId` (hex form of the token id).
   - `Proof` (challenge form or transport form) and `SignerBinding` (same key
     or instance certificate).
   - `ChallengeStore` (at most 1,000 challenges, 60 s life, single use).
   - `LeaseStore` (SQLite plus an in-memory index, deny list, distinct-subject
     count, survives restart). Each lease keeps the passport limits, and
     `lease_for(subject)` returns the lease (and limits) that apply.
   - `Registrar`: the five-step registration check of the change doc, in two
     calls: `check` (offline, writes nothing) and `grant` (deny, count,
     write), so the service runs the DHT revocation lookup between them.
   - `RequestLimiter` (10 requests per minute per source address).
   - Wire types of the two HTTPS endpoints, with the 16 KiB size check before
     any parse or signature check.
   - `RegisterError` with one variant for each refusal case, `Revoked`
     included (the service makes that one), and a stable `code()` string.
3. `roymctl coordinator passport issue`: writes a passport file and prints its
   id. The signer is always named (`--from` or `--node-key`).
4. Tests for every refusal case of `P-2`, and for `P-1`, `P-3`, `P-4` (lease
   end rule), `P-5` (store level), `P-6` (store level), `P-7`, `P-9` and the
   `issue` part of `P-8`, with both proof forms and both binding forms.
5. Written requirements on later slices: the service runs the DHT
   revocation check (B2, C8; D-B1-15), deny and list go through the running
   service (B2; D-B1-18), and the mailbox `register` request carries the
   instance certificate (C4, C8; D-B1-17).

Not in this slice: HTTP routes, the access function of the relay, TLS checks,
the DHT revocation lookup, `deny`/`leases` commands, the substrate side, the
mailbox `issue` variant (these are B2, B3 and C8).

## 2. Decisions this plan takes

Write each one into `## Deviations` of the operator-passport change doc as
`D-B1-<n>` when you implement it (the doc has an empty `## Deviations` now).

- **D-B1-1. The chain has exactly two tokens.** The leaf (child token) has
  exactly one proof (the passport). The passport has no proofs. Each token has
  exactly one capability. Anything else is `Malformed`. The change doc says
  "The chain has two tokens"; this makes it a rule. An operator who wants to
  admit a node directly still makes a passport for an owner, and the owner
  makes the child token. (owner check, see §4.3)
- **D-B1-2. The core is form-agnostic.** A `PassportUse` does not fix the
  proof form or the binding form. The caller passes a `Proof` and a
  `SignerBinding` for each registration. B2 always passes challenge form plus
  same key; C8 passes transport form plus instance certificate. The tests also
  run the two cross combinations, to show the two parts are independent.
- **D-B1-3. The challenge proof names its signer.** `Proof::Challenge` carries
  `signer_did`, and the signature is checked against that DID, not against
  the subject. The proven key is then `signer_did` in the challenge form and
  `verified_did` in the transport form. So the binding check has the same
  input in both forms, and "proven key is not bound to the subject" is a real,
  testable error in both forms.
- **D-B1-4. What the challenge signature covers.** The signer signs, with
  `Identity::sign_json` (RFC 8785 canonical JSON, the same as every other
  signed object in the code), this object:

  ```json
  {
    "domain": "syneroym-passport-challenge-v1",
    "service_did": "<host node DID>",
    "ability": "relay/home",
    "challenge": "<64 hex chars>",
    "token_hash": "<64 hex chars>"
  }
  ```

  `token_hash` is SHA-256 over the canonical JSON of the **whole presented
  child token, including its `proofs` and signatures**, not only its signed
  body. The signed body of a `CapabilityToken` leaves out `proofs`, so a hash
  of the body alone would let someone place a different passport under the
  same child token without a new signature. `ability` is extra domain
  separation between the two uses on one host. (owner check, see §4.4)
- **D-B1-5. The use limit counts running leases only.** `max_nodes` (and
  later `max_services`) counts distinct subjects that have a lease still
  running under this passport, not counting the registering subject itself.
  An expired lease frees its place. A subject that renews never counts
  against itself. (owner check, see §4.5)
- **D-B1-6. A missing count key means no limit.** ADR-0015 A10 says "a missing
  key means the open limit of the service applies". The relay has no open
  limit on nodes, so a passport without `max_nodes` has no node limit. For
  the mailbox, `PassportUse` has `open_count_limit: Option<u64>`; it is `None`
  for both uses now, and C8 may set it.
- **D-B1-7. Facts are checked at issue, read loosely at registration.**
  `issue_passport` refuses a facts key that is not a reserved key of the use,
  and a value that is not a whole number of at least 1. At registration the
  core reads only the count key of the use; an unknown key is ignored. A count
  key that is present but not a whole number is `Malformed`. A value of 0
  admits nobody (`LimitReached`).
- **D-B1-8. The lease end.** `lease_ends_at = min(child.expires_at_secs,
  passport.expires_at_secs, now + use.max_lease_secs)`. The change doc names
  only the child token and `max_lease_secs`. `verify_chain` does not check
  that a child expires before its parent, so the passport expiry is added
  here. `child_token` also clamps the child expiry to the passport expiry.
- **D-B1-9. Deny removes the leases.** `deny(id)` writes the id to the deny
  table and deletes every lease row of that passport, in one transaction. The
  admission check then reads leases only. A later registration with a denied
  passport is refused with `Denied`. A deny of an id with no leases is still
  stored (deny in advance). There is no "undeny" (backlog row).
- **D-B1-10. One SQLite file per service, plain (not encrypted).** The lease
  table holds passport ids, DIDs and times: no secret. `LeaseStore::open(dir,
  db_name)` follows `AlertStore::open` (`crates/app_orchestration/src/alerts.rs`):
  WAL mode, `CREATE TABLE IF NOT EXISTS`, no version ladder. B2 picks the file
  name and directory.
- **D-B1-11. Clock as a parameter.** Every check, and every function that
  makes a token, takes `now_secs: u64`: `issue_passport`, `child_token`,
  `Registrar::check`, `Registrar::grant`, the stores. The callers (B2, B3,
  C8, `roymctl`) pass `syneroym_passport::now_secs()`, a one-line wall-clock
  helper in the crate (step 4). The tests read the clock once (`Fixture::now`) and
  use only that value, so a test cannot fail when the clock moves one second
  between two reads. The one exception is `DelegationCertificate::issue`,
  which reads the wall clock itself and whose signed bytes are private to
  `crates/identity`: the tests give each certificate case a margin of at
  least 5 seconds on each side of the boundary (see step 11).
- **D-B1-12. The request limiter lives in the core.** task.md lists "request
  limits" in X4. `RequestLimiter` is a small fixed-window counter keyed by
  `IpAddr`, in memory. It keys on the full address; grouping IPv6 by `/64` is
  a backlog row. It tracks at most 10,000 addresses; when full it first drops
  windows that have ended, and if it is still full it refuses the new address
  (fails closed).
- **D-B1-13. Sync API.** `LeaseStore` and `Registrar` are synchronous and
  `Send + Sync`. The SQLite write is short. B2 may call `grant` inside
  `spawn_blocking`. `LeaseStore::is_admitted` reads only memory, so the Iroh
  relay access function (an async closure,
  `iroh-relay-0.97.0/src/server.rs:139`) can call it directly.
- **D-B1-14. `roymctl` flag names.** `roymctl coordinator passport issue
  (--from <name> | --node-key) --to <owner did> --expires-days <n>
  [--max-nodes <n>] [--out <file>]`, with the global `--substrate <did>` for
  the host node DID. `--from` and `--node-key` form a required group, with
  exactly one of them: the command never picks the signer by itself (see
  D-B1-21). The names follow the existing token command `identity
  issue-grant` (`--from`, `--to`, `--expires-days`). The change doc proposed
  `--audience`, `--expires-in <duration>`, and no signer flag. The workspace
  has no duration parser, and `roymctl` already uses two expiry units:
  `--expires-days` (`identity issue-grant`) and `--expires-hours` (`identity
  certify-*`, `roym transaction`). Days fit a passport that lives weeks or
  months. (owner check, see §4.6)
- **D-B1-15. DHT revocation: checked by the service, between `check` and
  `grant`.**
  ADR-0015 §2 and A7 require the DHT revocation check on every token in the
  chain. The router already has it for UCAN chains:
  `ucan_chain_not_revoked` (`crates/router/src/route_handler/io.rs:69`). For
  each `(issuer, audience)` edge, it reads the issuer's master anchor and
  refuses the chain when the audience is in its `revoked_keys`. Two edges
  matter here: `(operator, owner)` lets the operator cut one owner, and
  `(owner, subject)` lets the owner cut one node or one member master. That
  second edge closes the gap "an owner cannot cut one node". The check is a
  network lookup, and the core is synchronous and offline (D-B1-13). So the
  core does not run the lookup. Instead:
  - B1 adds the error variant `RegisterError::Revoked` (code `"revoked"`),
    so the error set and the wire codes are final now.
  - **The lookup runs after every offline check, never before.** Each lookup
    can wait up to 5 seconds, and the DIDs it looks up come from the request,
    so the sender chooses them. If the lookup ran first, anyone could make the
    service do two network lookups per request with no valid signature at
    all; the only limit would be 10 requests per minute per address. The
    router uses the same order: it verifies the chain first, then checks
    revocation (`crates/router/src/route_handler/io.rs:231`).
  - So B1 splits registration into two calls, and there is no single
    `register`:
    1. `Registrar::check(token, proof, binding, now) -> CheckedRegistration`:
       every offline check (shape, proof of possession, binding, chain,
       limit value). It reads memory only and writes nothing, except that
       it uses up the challenge.
    2. The service runs `ucan_chain_not_revoked(checked.token(), resolver)`
       and refuses with `RegisterError::Revoked`.
    3. `Registrar::grant(checked, now) -> Lease`: the deny check, the use
       limit count and the write, under one lock.
    Only a caller whose proof and chain are already valid reaches the
    network. There is no combined call, so the revocation step is easy to
    see at every call site. The split does not force it: a service can still
    call `check` and then `grant` with no lookup between them. Code review of
    B2 and C8 must check that the lookup is there.
  - B2 and C8 must do step 2. B2 moves `ucan_chain_not_revoked` to a place
    both can use, or makes it public. That is not B1 work.
  - A revocation stops the *next* registration. A lease that is still
    running stays until it ends (at most 24 hours for the relay). This
    matches how the deny list ends access (`P-4`).
  (owner check, see §4.11)
- **D-B1-16. The lease keeps the passport limits.** The lease row stores the
  passport facts as JSON (`limits`) and the time of the registration
  (`registered_at`). A mailbox therefore still knows the raised quota of a
  queue after a restart. `LeaseStore::lease_for(subject, now)` returns the
  lease that applies to a subject. When a subject holds running leases under
  two passports, the **newest registration wins**: the lease with the
  largest `registered_at`, then the largest `lease_ends_at`, then the
  smallest `passport_id`, so the choice is the same every time. The newest
  registration is the passport the receiver chose last. The other choices
  are worse. "Largest limit for each key" mixes two operator decisions into
  one quota that neither operator signed. "Latest lease end" changes with
  each renewal.
- **D-B1-17. Requirement on C4 and C8: the mailbox `register` request
  carries the whole instance certificate.** The router keeps only
  `VerifiedIdentity { master_did, temporary_did }`
  (`crates/router/src/handshake.rs:16`), and drops the certificate. It also
  accepts the `routing` scope as well as `service-instance`
  (`TRANSPORT_SCOPES`). So `SignerBinding::InstanceCertificate` cannot be
  built from the transport alone. C4 and C8 put the certificate into the
  `register` request body. M-3 already says the caller "shows an instance
  certificate", so this is allowed. The mailbox then passes
  `Proof::Transport { verified_did: <the transport's temporary_did> }` and
  `SignerBinding::InstanceCertificate(<the certificate from the body>)`.
  The core then checks the `service-instance` scope; the transport does not.
  The core check also requires `cert.temporary_did == verified_did`, so a
  caller cannot show another instance's certificate. C8 must not add a
  "the transport already checked it" binding form. Write this into the
  service-mailbox change doc in step 15.
- **D-B1-18. Requirement on B2: `deny` and `leases` go through the running
  service.** `LeaseStore` loads its in-memory index only at `open`. A second
  process that writes the SQLite file directly would not reach the running
  relay until it restarts, and that fails `P-6`. So `roymctl coordinator
  passport deny` and `leases` must call the process that owns the store
  (for example through the control plane, the same path B3 uses for the
  child-token store). B1 adds no reload method. A reload needs a trigger
  (a file watch or a poll). It also opens a window where the file says
  "denied" and the memory still admits. And it would let two writers race
  on the use-limit count, which `grant` keeps atomic only inside one
  process. The `LeaseStore` doc comment says "one process owns the file".
- **D-B1-19. A passport that forbids delegation is refused explicitly.**
  `verify_chain` asks `is_trusted_root(issuer, capability)` for **every**
  token. When the owner is also an accepted root (a self-hosted relay, where
  the controller owns the substrate too), the child token is rooted on its
  own. Then `verify_chain` does not need the passport, and a
  `can_delegate: false` caveat on the passport has no effect. A passport
  always needs a child token, so a passport that forbids delegation can
  never be used. `check_chain` therefore refuses it before `verify_chain`:
  `if !passport.capabilities[0].can_delegate() { InvalidChain("the passport
  forbids delegation") }`. Then the result is the same whether or not the
  owner is a root. The passport signature is still verified in both cases:
  `verify_chain` verifies every proof's signature even when the leaf is
  rooted (`granted_capabilities` runs on each proof). A test pins that too.
  `issue_passport` never sets caveats.
- **D-B1-20. DIDs in a lease are in canonical form.** A lease is keyed on the
  child token's `audience_did` string. B2 will look it up with
  `derive_did_key(<endpoint public key>)`. The z-base-32 decoder probably
  accepts other spellings of the same key: 34 bytes need 272 bits, and 55
  characters hold 275, so the last character has 3 spare bits. Not tested.
  So the core refuses a subject that is not canonical:
  `Malformed` unless `derive_did_key(&resolve_did_key(s)?) == s`. The same
  check applies to `audience_did` in `issue_passport` and `subject_did` in
  `child_token`, so a token with a non-canonical DID is never made. Issuer
  DIDs are always canonical, because `sign` derives them from the key. A
  non-canonical `signer_did` in a challenge proof cannot get through: it
  fails `SameKey` (string compare) as `NotBound`.
- **D-B1-21. `roymctl issue` never signs with the node key by default.**
  Which roots a service accepts is still open (§4.7). A default of
  `substrate.key` would decide it: if B2 follows ADR-0015 A10.4, those
  passports would be refused on every substrate with a controller. So the
  signer is always named: `--from <name>` (a local identity, normally the
  controller) or `--node-key` (the help text says: "only for a host
  substrate that has no controller").
- **D-B1-22. The crate has its own `now_secs()`.** Every caller of the crate
  (B2's coordinator, B3's substrate, C8's mailbox, `roymctl`) needs the wall
  clock in Unix seconds. `syneroym_passport::now_secs()` gives it in one
  line. `syneroym_roym_core::clock::now_secs` (`crates/roym_core/src/clock.rs:18`)
  has the same body, but it is the clock of the Roym product, built for
  `wasm32-wasip2` guests ("the one wall-clock read in the product"). The
  coordinator and the substrate are platform code. They must not depend on
  the product crate for a clock. `roymctl` could use either; it uses the
  passport one, so all passport callers read the same function.

## 3. Step-by-step changes

All changes add code. No existing function changes its signature, so no
existing call site needs an update. The only edits to existing code are:
`CapabilityToken::issue_with_anchor` (internal refactor, same signature),
`roymctl`'s `Commands` enum and `run`, and the visibility of one `roymctl`
helper.

### Step 1 — abilities (`crates/ucan/src/capability.rs`)

Add two constants to `impl Ability`, in alphabetical place with the others:

```rust
/// Keep a mailbox queue at this mailbox service (ADR-0015 A10). Flat: it
/// entails only itself, and is entailed by `substrate/admin` like
/// everything else.
pub const MAILBOX_USE: &'static str = "mailbox/use";
/// Use this relay as the home relay (ADR-0015 A10). Flat, like
/// `MAILBOX_USE`.
pub const RELAY_HOME: &'static str = "relay/home";
```

Do not add them to `Ability::tier`: they are flat.

Tests, in the existing `#[cfg(test)] mod tests` of `capability.rs` (the file
is 598 lines; this adds ~25):

- `passport_abilities_are_flat`: `relay/home` does not entail `mailbox/use`,
  and the reverse; neither entails `data-layer/read`; `data-layer/admin`
  entails neither.
- `substrate_admin_entails_passport_abilities`: `substrate/admin` entails
  both. (This is why the passport check in step 9 must compare exactly and
  not use `entails`.)

### Step 2 — public `sign`, and refactor `issue_with_anchor` (`crates/ucan/src/token.rs`)

`child_token` (step 6) needs an absolute expiry: "now + seconds" computed
twice (once by the caller, once inside `issue`) can be one second apart, and
the child would outlive the passport by one second. All fields of
`CapabilityToken` are already `pub`, so add one method that signs a token
built field by field:

```rust
/// Sign a token built field by field. `issuer_did` must already be the DID
/// of `issuer`; a token signed by another key would never verify.
pub fn sign(&mut self, issuer: &Identity) -> Result<()> {
    let did = substrate::derive_did_key(&issuer.public_key());
    if did != self.issuer_did {
        return Err(anyhow!("issuer_did {} is not the DID of the signing key {did}", self.issuer_did));
    }
    self.signature = issuer.sign_json(&self.signing_value())?;
    Ok(())
}
```

Change the last two lines of `issue_with_anchor` to
`token.sign(issuer)?; Ok(token)`. Same behaviour, one signing path.

### Step 3 — passport id (`crates/ucan/src/token/id.rs`, new)

`crates/ucan/src/token/tests.rs` is on the oversized list at 1,087 lines
(`xtask/oversized-test-files.txt:33`). Do not add a single line to it — not
even a `mod` line. Put the id in its own child module with its own tests:

- `token.rs`: add `mod id;` next to `#[cfg(test)] mod tests;`.
- `crates/ucan/src/token/id.rs`:

```rust
//! The id of a token: the SHA-256 of the canonical body its signature covers
//! (ADR-0015 A10). Deny lists and use counts key on it.

use anyhow::Result;
use sha2::{Digest, Sha256};
use syneroym_identity::substrate;

use super::CapabilityToken;

impl CapabilityToken {
    /// SHA-256 of the RFC 8785 canonical JSON of the signed body: every
    /// field except `signature` and `proofs`. Two tokens with the same id
    /// carry the same signed claims. The id does not change when the proofs
    /// change.
    pub fn token_id(&self) -> Result<[u8; 32]> {
        let canonical = substrate::canonicalize_json_value(&self.signing_value());
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(Sha256::digest(&bytes).into())
    }
}

#[cfg(test)]
mod tests;
```

- `crates/ucan/src/token/id/tests.rs`:
  - `token_id_is_stable_for_the_same_token` (two calls, same bytes; and a
    serde round trip gives the same id).
  - `token_id_changes_when_a_signed_field_changes` (change `facts`, then
    `expires_at_secs`; each gives another id).
  - `token_id_ignores_proofs_and_signature`.
  - `token_id_is_the_hash_of_the_signed_bytes`: compute
    `Sha256(serde_json::to_string(canonicalize(signing_value)))` by hand and
    compare. This pins the definition: `sign_json` signs exactly these bytes
    (`crates/identity/src/keys.rs:203`).
- `crates/ucan/Cargo.toml`: add `sha2.workspace = true`.

Export nothing new from `lib.rs` (the method is on an exported type).

### Step 4 — the crate skeleton

New directory `crates/passport/`. It joins the workspace by the
`members = ["crates/*", ...]` glob.

`crates/passport/Cargo.toml`:

```toml
[package]
name = "syneroym-passport"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
syneroym-identity.workspace = true
syneroym-ucan.workspace = true
anyhow.workspace = true
getrandom.workspace = true
hex.workspace = true
rusqlite.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

Root `Cargo.toml`, `[workspace.dependencies]`: add
`syneroym-passport = { path = "crates/passport" }` next to `syneroym-ucan`.

Files (each with its tests in `<module>/tests.rs`, split into
`<module>/tests/<topic>.rs` if a test file would pass 800 lines):

| File | Contents |
| --- | --- |
| `src/lib.rs` | crate doc, `mod` lines, re-exports |
| `src/passport_use.rs` | `PassportUse`, `RELAY_HOME`, `MAILBOX_USE`, facts key constants |
| `src/issue.rs` | `issue_passport`, `child_token`, `PassportId` |
| `src/error.rs` | `RegisterError` |
| `src/proof.rs` | `Challenge`, `Proof`, `SignerBinding`, `challenge_message`, `sign_challenge`, `presentation_hash` |
| `src/challenge_store.rs` | `ChallengeStore` |
| `src/lease_store.rs` | `LeaseStore`, `Lease` (and `lease_store/cache.rs` for the in-memory index) |
| `src/registrar.rs` | `Registrar`, the five checks |
| `src/limiter.rs` | `RequestLimiter` |
| `src/wire.rs` | `ChallengeResponse`, `RegisterRequest`, `RegisterResponse`, `ErrorBody`, `MAX_REQUEST_BYTES` |

`src/lib.rs`:

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Operator passports (ADR-0015 A10): a signed, time-limited token that
//! admits a user to an operator's service (a home relay or a dedicated
//! mailbox). This crate holds the parts both uses share: issuing, the
//! registration checks, the challenge store, the lease table with its deny
//! list, and the request limiter. A service adds only its own HTTP or RPC
//! surface and how it enforces a lease.

mod challenge_store;
mod error;
mod issue;
mod lease_store;
mod limiter;
mod passport_use;
mod proof;
mod registrar;
pub mod wire;

pub use challenge_store::{CHALLENGE_LIFE_SECS, ChallengeStore, MAX_CHALLENGES};
pub use error::RegisterError;
pub use issue::{PassportId, child_token, issue_passport};
pub use lease_store::{Lease, LeaseStore};
pub use limiter::{RequestLimiter, REQUESTS_PER_MINUTE};
pub use passport_use::{MAILBOX_USE, PassportUse, RELAY_HOME, facts};
pub use proof::{Challenge, Proof, SignerBinding, presentation_hash, sign_challenge};
pub use registrar::{CheckedRegistration, Registrar};

// with `use std::time::{SystemTime, UNIX_EPOCH};` at the top of lib.rs

/// Unix seconds from the wall clock, for the `now_secs` argument of every
/// call in this crate. Saturates to 0 before the epoch instead of failing.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
```

Lint notes for every file: the workspace sets `missing_debug_implementations
= "warn"`, so every public type derives or implements `Debug`.
`expect_used`/`unwrap_used` are warnings: take a lock with
`.lock().unwrap_or_else(PoisonError::into_inner)` (same for `RwLock::read`/
`write`), so no `allow` is needed. Avoid `as` casts between `u64` and `i64`;
use `i64::try_from` and map the error.

### Step 5 — `PassportUse` (`src/passport_use.rs`)

```rust
/// Reserved facts keys of ADR-0015 A10.
pub mod facts {
    pub const MAX_NODES: &str = "max_nodes";
    pub const MAX_SERVICES: &str = "max_services";
    pub const MAX_BYTES: &str = "max_bytes";
    pub const MAX_MESSAGES: &str = "max_messages";
    pub const MAX_TTL_SECS: &str = "max_ttl_secs";
}

/// What one use of the passport fixes: its ability, its resource selector,
/// its signed use limits and how long one lease may last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassportUse {
    /// The exact ability a passport for this use must name.
    pub ability: &'static str,
    /// The selector after `substrate:<host node did>/`.
    pub selector: &'static str,
    /// The facts key that limits distinct subjects for one passport.
    pub count_key: &'static str,
    /// Every facts key a passport for this use may carry.
    pub limit_keys: &'static [&'static str],
    /// The subject limit when the passport has no `count_key`. `None`: no limit.
    pub open_count_limit: Option<u64>,
    /// The longest one lease may last.
    pub max_lease_secs: u64,
}

pub const RELAY_HOME: PassportUse = PassportUse {
    ability: Ability::RELAY_HOME,
    selector: "relay",
    count_key: facts::MAX_NODES,
    limit_keys: &[facts::MAX_NODES],
    open_count_limit: None,
    max_lease_secs: 24 * 60 * 60,
};

pub const MAILBOX_USE: PassportUse = PassportUse {
    ability: Ability::MAILBOX_USE,
    selector: "mailbox",
    count_key: facts::MAX_SERVICES,
    limit_keys: &[facts::MAX_SERVICES, facts::MAX_BYTES, facts::MAX_MESSAGES, facts::MAX_TTL_SECS],
    open_count_limit: None,
    // The same as the mailbox queue lifetime, so a sleeping receiver keeps
    // its raised quota while its queue lives.
    max_lease_secs: 30 * 24 * 60 * 60,
};

impl PassportUse {
    /// `substrate:<host_node_did>/<selector>` (ADR-0015 A1 selector form).
    #[must_use]
    pub fn resource(&self, host_node_did: &str) -> ResourceUri {
        ResourceUri(format!("substrate:{host_node_did}/{}", self.selector))
    }

    /// The one capability a passport and its child token carry.
    #[must_use]
    pub fn capability(&self, host_node_did: &str) -> Capability {
        Capability { with: self.resource(host_node_did), can: Ability(self.ability.to_string()), caveats: None }
    }

    /// Whether `cap` names exactly this use's ability and resource.
    /// Entailment is not accepted (ADR-0015 A10.1): `substrate/admin`, or a
    /// bare `substrate:<did>`, is never a passport. Caveats are not compared
    /// here; `verify_chain` already applies `can_delegate`.
    #[must_use]
    pub fn names_exactly(&self, cap: &Capability, host_node_did: &str) -> bool {
        cap.can.0 == self.ability && cap.with == self.resource(host_node_did)
    }

    /// The same use with another lease limit (B2 may read it from config).
    #[must_use]
    pub const fn with_max_lease_secs(mut self, secs: u64) -> Self {
        self.max_lease_secs = secs;
        self
    }
}
```

Tests (`passport_use/tests.rs`): resource string form; `names_exactly`
refuses `substrate/admin`, the bare `substrate:<did>`, another host DID,
`relay/x` selectors deeper than `relay`, and the other use's ability.

### Step 6 — issue and child token (`src/issue.rs`)

```rust
/// The id of a passport: hex of `CapabilityToken::token_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PassportId(String);

impl PassportId {
    pub fn of(passport: &CapabilityToken) -> anyhow::Result<Self> {
        Ok(Self(hex::encode(passport.token_id()?)))
    }
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}
// FromStr / TryFrom<String>: exactly 64 lowercase hex chars, else an error
// "a passport id is 64 lowercase hex characters". Display prints the hex.
// From<PassportId> for String.
```

```rust
/// Issue a passport: the operator root grants `use_` on `host_node_did` to
/// `audience_did` (the user's owner DID), with signed use limits.
pub fn issue_passport(
    operator: &Identity,
    host_node_did: &str,
    use_: &PassportUse,
    audience_did: &str,
    limits: Map<String, Value>,
    expires_in_secs: u64,
    now_secs: u64,
) -> anyhow::Result<CapabilityToken>
```

Pseudo-code:

```text
for (key, value) in &limits:
    ensure use_.limit_keys contains key,
        else "<key> is not a use limit of <ability>; allowed: <limit_keys>"
    ensure value.as_u64() is Some(n) with n >= 1,
        else "<key> must be a whole number of at least 1"
ensure expires_in_secs > 0
ensure_canonical_did(audience_did)?           // D-B1-20
ensure_canonical_did(host_node_did)?
token = CapabilityToken {
    issuer_did: derive_did_key(operator.public_key()), audience_did, anchor_did: None,
    capabilities: vec![use_.capability(host_node_did)], facts: limits,
    not_before_secs: now_secs,
    expires_at_secs: now_secs.checked_add(expires_in_secs)?,   // error on overflow
    proofs: vec![], signature: String::new(),
}
token.sign(operator)?                        // step 2
Ok(token)
```

`ensure_canonical_did` is a crate-private helper in `issue.rs`, also used by
the registrar:

```rust
/// A DID is canonical when it is exactly what `derive_did_key` makes for its
/// key. Another spelling of the same key is refused, because leases and
/// lookups compare DIDs as strings.
pub(crate) fn ensure_canonical_did(did: &str) -> anyhow::Result<()> {
    let key = substrate::resolve_did_key(did)?;
    anyhow::ensure!(substrate::derive_did_key(&key) == did, "{did} is not in canonical form");
    Ok(())
}
```

```rust
/// The owner re-delegates `passport` to `subject_did` (the node DID for the
/// relay, the member master DID for the mailbox). The child carries the same
/// capability, no facts (the service reads limits from the passport only),
/// and never outlives the passport.
pub fn child_token(
    owner: &Identity,
    passport: &CapabilityToken,
    subject_did: &str,
    expires_in_secs: u64,
    now_secs: u64,
) -> anyhow::Result<CapabilityToken>
```

Pseudo-code:

```text
owner_did = derive_did_key(owner.public_key())
ensure passport.audience_did == owner_did,
    else "this passport was issued to <aud>, not to the owner identity <owner_did>"
ensure passport.proofs.is_empty() && passport.capabilities.len() == 1,
    else "this token is not a passport"
ensure now_secs < passport.expires_at_secs, else "the passport has expired"
ensure_canonical_did(subject_did)?
expires_at = min(now_secs.saturating_add(expires_in_secs), passport.expires_at_secs)
token = CapabilityToken {
    issuer_did: owner_did, audience_did: subject_did, anchor_did: None,
    capabilities: passport.capabilities.clone(), facts: Map::new(),
    not_before_secs: now_secs, expires_at_secs: expires_at,
    proofs: vec![passport.clone()], signature: String::new(),
}
token.sign(owner)?          // step 2
Ok(token)
```

Tests (`issue/tests.rs`):

- `issue_refuses_a_limit_key_of_another_use` (`max_bytes` on `RELAY_HOME`).
- `issue_refuses_a_limit_that_is_not_a_positive_whole_number` (0, -1, 1.5,
  `"3"`).
- `issued_passport_names_the_selector_resource`.
- `child_token_never_outlives_the_passport` (child asks for 10× the passport
  life; its `expires_at_secs == passport.expires_at_secs`).
- `child_token_refuses_a_passport_for_another_owner`.
- `child_token_refuses_an_expired_passport` (pass `now` after expiry).
- `child_token_has_no_facts`.
- `issued_passport_uses_the_given_clock` (`not_before_secs == now`,
  `expires_at_secs == now + life`; pass a `now` far from the wall clock to
  show the wall clock is not read).
- `a_non_canonical_did_is_refused`: take a canonical DID, change its last
  z-base-32 character to another one that decodes to the same 34 bytes. Find
  it by trying each of the 32 alphabet characters and keeping one where
  `resolve_did_key` gives the same key. If no such character exists, the
  test asserts that and the D-B1-20 check stays as a cheap guard. Both
  `issue_passport` (audience) and `child_token` (subject) refuse the other
  spelling.
- `passport_id_round_trips_and_rejects_bad_hex` (63 chars, uppercase,
  non-hex).

### Step 7 — `RegisterError` (`src/error.rs`)

One variant for each refusal case. `code()` is the stable string B2 puts in
the HTTP error body and C8 in the RPC error.

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegisterError {
    #[error("request body is larger than {max} bytes")]
    RequestTooLarge { max: usize },                 // "request_too_large"
    #[error("malformed request: {0}")]
    Malformed(String),                              // "malformed"
    #[error("unknown challenge: never issued, already used, or dropped")]
    UnknownChallenge,                               // "unknown_challenge"
    #[error("the challenge is older than {} seconds", CHALLENGE_LIFE_SECS)]
    ChallengeExpired,                               // "challenge_expired"
    #[error("the challenge signature does not verify")]
    BadSignature,                                   // "bad_signature"
    #[error("the proven key may not act for the subject")]
    NotBound,                                       // "not_bound"
    #[error("the passport is not issued by an accepted operator root")]
    UntrustedRoot,                                  // "untrusted_root"
    #[error("the child token is not issued by the passport audience")]
    BrokenChain,                                    // "broken_chain"
    #[error("the token does not name ability {ability} on {resource} exactly")]
    WrongCapability { ability: String, resource: String }, // "wrong_capability"
    #[error("the passport or the child token has expired")]
    Expired,                                        // "expired"
    #[error("the token chain is not valid: {0}")]
    InvalidChain(String),                           // "invalid_chain"
    #[error("the passport is denied")]
    Denied,                                         // "denied"
    /// Made by the service, not by the core: the DHT revocation check runs
    /// between `Registrar::check` and `Registrar::grant` (D-B1-15). Defined
    /// here so all services share one error set and one set of wire codes.
    #[error("a key in the token chain is revoked by its issuer")]
    Revoked,                                        // "revoked"
    #[error("the passport admits at most {limit} subjects")]
    LimitReached { limit: u64 },                    // "limit_reached"
    #[error("lease store error: {0}")]
    Store(String),                                  // "internal"
}

impl RegisterError {
    #[must_use]
    pub const fn code(&self) -> &'static str { /* table above */ }
}
```

Keep `code()` a plain `match` with one-line arms. A unit test checks that all
codes are distinct (build the list of one value of each variant; collect into
a `HashSet`; compare lengths).

### Step 8 — proofs and binding (`src/proof.rs`)

```rust
/// A fresh random value the caller must sign (challenge form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Challenge([u8; 32]);

impl Challenge {
    /// 32 bytes from `getrandom::fill`. Returns an error instead of a
    /// panic; `ChallengeStore::issue` passes it on.
    pub(crate) fn random() -> anyhow::Result<Self>
    pub fn to_hex(&self) -> String
    pub fn from_hex(s: &str) -> Result<Self, RegisterError>  // Malformed on bad hex / length
}

/// How the caller proves it holds a key.
#[derive(Debug, Clone)]
pub enum Proof {
    /// Plain HTTPS (relay): `signer_did` signed the challenge message.
    Challenge { challenge: Challenge, signer_did: String, signature: String },
    /// Iroh (mailbox): the transport already verified this key.
    Transport { verified_did: String },
}

/// May the proven key act for the subject?
#[derive(Debug, Clone)]
pub enum SignerBinding {
    /// The proven key is the subject (relay: the node key).
    SameKey,
    /// The proven key is an instance key, bound to the subject (a member
    /// master DID) by its service-instance certificate (ADR-0016). The
    /// caller sends the whole certificate in its request: the router keeps
    /// only the two DIDs (D-B1-17). This check is the only one that
    /// requires the `service-instance` scope.
    InstanceCertificate(DelegationCertificate),
}

pub const CHALLENGE_DOMAIN: &str = "syneroym-passport-challenge-v1";

/// SHA-256 of the canonical JSON of the whole presented token, proofs and
/// signatures included.
pub fn presentation_hash(token: &CapabilityToken) -> anyhow::Result<[u8; 32]>

pub(crate) fn challenge_message(
    host_node_did: &str, ability: &str, challenge: &Challenge, token_hash: &[u8; 32],
) -> Value   // the JSON object of D-B1-4

/// Sign a challenge for registration at `host_node_did` (the substrate side
/// calls this in B3; the tests call it here).
pub fn sign_challenge(
    signer: &Identity, host_node_did: &str, use_: &PassportUse,
    challenge: &Challenge, token: &CapabilityToken,
) -> anyhow::Result<String>
// = signer.sign_json(&challenge_message(host_node_did, use_.ability, challenge,
//                                        &presentation_hash(token)?))
```

`SignerBinding::check`:

```rust
impl SignerBinding {
    pub(crate) fn check(&self, proven_did: &str, subject: &str, now_secs: u64) -> Result<(), RegisterError> {
        let bound = match self {
            Self::SameKey => proven_did == subject,
            Self::InstanceCertificate(cert) => {
                cert.temporary_did == proven_did
                    && cert.master_did == subject
                    && cert.verify_at(subject, &[delegation::SCOPE_SERVICE_INSTANCE], now_secs).is_ok()
            }
        };
        if bound { Ok(()) } else { Err(RegisterError::NotBound) }
    }
}
```

`Proof::proven_did` (private to the crate, called by the registrar):

```text
fn verify(&self, challenges, host_node_did, use_, token, now) -> Result<&str, RegisterError>:
    Challenge { challenge, signer_did, signature }:
        challenges.take(challenge, now)?       // UnknownChallenge | ChallengeExpired
        // Taken before the signature check: a wrong signature still uses up
        // the challenge, so one challenge allows one guess.
        hash = presentation_hash(token).map_err(Malformed)?
        msg  = challenge_message(host_node_did, use_.ability, challenge, &hash)
        substrate::verify_json_signature(signer_did, &msg, signature)
            .map_err(|_| BadSignature)?
        Ok(signer_did)
    Transport { verified_did }:
        Ok(verified_did)
```

### Step 9 — `ChallengeStore` (`src/challenge_store.rs`)

```rust
pub const MAX_CHALLENGES: usize = 1_000;
pub const CHALLENGE_LIFE_SECS: u64 = 60;

#[derive(Debug, Default)]
pub struct ChallengeStore { inner: Mutex<Queue> }

#[derive(Debug, Default)]
struct Queue {
    order: VecDeque<(Challenge, u64)>,   // issue order; may hold taken entries
    live: HashMap<Challenge, u64>,       // challenge -> issued_at
}

impl ChallengeStore {
    pub fn issue(&self, now_secs: u64) -> anyhow::Result<Challenge>;
    pub(crate) fn take(&self, challenge: &Challenge, now_secs: u64) -> Result<(), RegisterError>;
    #[cfg(test)] fn live_len(&self) -> usize;
}
```

Pseudo-code:

```text
issue(now):
    c = Challenge::random()?
    q = lock
    // drop challenges whose life has ended
    while let Some(&(old, at)) = q.order.front() && now >= at + LIFE:
        q.order.pop_front(); if q.live.get(&old) == Some(&at) { q.live.remove(&old) }
    // keep at most MAX_CHALLENGES: the oldest live one goes first
    if q.live.len() >= MAX_CHALLENGES:
        while let Some((old, _)) = q.order.pop_front():
            if q.live.remove(&old).is_some() { break }
    // taken challenges stay in `order` until their life ends; compact so
    // `order` cannot grow without bound under fast issue-and-take
    if q.order.len() >= 2 * MAX_CHALLENGES:
        q.order.retain(|(c, _)| q.live.contains_key(c))   // borrow: collect live keys first
    q.order.push_back((c, now)); q.live.insert(c, now)
    Ok(c)

take(c, now):
    q = lock
    match q.live.remove(c):
        None => Err(UnknownChallenge)
        Some(at) if now >= at + LIFE => Err(ChallengeExpired)
        Some(_) => Ok(())
```

Tests (`challenge_store/tests.rs`):

- `a_challenge_works_once` (second `take` → `UnknownChallenge`).
- `a_challenge_expires_after_its_life` (`take` at `now + 60` → `ChallengeExpired`;
  at `now + 59` → Ok, on another challenge).
- `the_oldest_challenge_is_dropped_first` (issue 1,001; the first →
  `UnknownChallenge`, the second and the last → Ok).
- `the_store_never_holds_more_than_the_limit` (issue 5,000; `live_len() <= 1000`).
- `issue_and_take_does_not_grow_without_bound` (10,000 issue+take pairs at one
  `now`; the internal `order` length stays `< 2 * MAX_CHALLENGES + 1`).
- `an_unknown_challenge_is_refused`.

### Step 10 — `LeaseStore` (`src/lease_store.rs`, `src/lease_store/cache.rs`)

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub passport_id: PassportId,
    pub owner_did: String,
    pub subject: String,
    pub lease_ends_at: u64,
    /// When this lease was last written (first registration or renewal).
    pub registered_at: u64,
    /// The signed facts of the passport (never of the child token). The
    /// mailbox reads its raised quotas from these.
    pub limits: Map<String, Value>,
}

/// The lease table and deny list of one service. SQLite holds them across
/// restarts; an in-memory index answers admission checks without I/O. One
/// process owns the file: the index is loaded only at `open`, so a write by
/// another process is not seen until a restart. Deny and list therefore go
/// through the running service.
#[derive(Debug, Clone)]
pub struct LeaseStore { inner: Arc<Inner> }

#[derive(Debug)]
struct Inner {
    conn: Mutex<Connection>,   // also serialises writers, so count-then-insert is atomic
    cache: RwLock<Cache>,
}
```

Schema (in `init_schema`, unconditional `IF NOT EXISTS`, like `AlertStore`):

```sql
CREATE TABLE IF NOT EXISTS passport_leases (
    passport_id   TEXT NOT NULL,
    owner_did     TEXT NOT NULL,
    subject       TEXT NOT NULL,
    lease_ends_at INTEGER NOT NULL,
    registered_at INTEGER NOT NULL,
    limits        TEXT NOT NULL,   -- the passport facts, JSON object
    PRIMARY KEY (passport_id, subject)
);
CREATE INDEX IF NOT EXISTS idx_passport_leases_end ON passport_leases(lease_ends_at);
CREATE TABLE IF NOT EXISTS passport_denied (
    passport_id TEXT PRIMARY KEY,
    denied_at   INTEGER NOT NULL
);
```

`cache.rs` (crate-private):

```rust
#[derive(Debug, Default)]
pub(super) struct Cache {
    by_passport: HashMap<PassportId, HashMap<String, Lease>>, // passport -> subject -> lease
    by_subject: HashMap<String, HashSet<PassportId>>,         // subject -> passports it has leases under
    denied: HashSet<PassportId>,
}
// methods: insert(lease), remove_passport(id) -> usize, prune(now),
// is_admitted(subject, now) -> bool,
// lease_for(subject, now) -> Option<Lease>:
//     the running leases of `subject` (via by_subject -> by_passport), and of
//     those the one with the largest registered_at, then the largest
//     lease_ends_at, then the smallest passport_id (D-B1-16),
// other_running_subjects(id, subject, now) -> u64,
// running(now) -> Vec<Lease> (sorted by passport_id, then subject),
// deny(id), is_denied(id)
```

Public API:

```rust
impl LeaseStore {
    /// Open (or create) `<dir>/<db_name>` and load the running leases and
    /// the deny list. Rows whose lease has ended are deleted.
    pub fn open(dir: impl AsRef<Path>, db_name: &str, now_secs: u64) -> anyhow::Result<Self>;
    pub fn open_in_memory() -> anyhow::Result<Self>;
    /// Whether `subject` has a running lease. Reads memory only.
    #[must_use] pub fn is_admitted(&self, subject: &str, now_secs: u64) -> bool;
    /// The running lease that applies to `subject`, with its limits. With
    /// leases under several passports, the newest registration wins.
    /// Reads memory only.
    #[must_use] pub fn lease_for(&self, subject: &str, now_secs: u64) -> Option<Lease>;
    #[must_use] pub fn running_leases(&self, now_secs: u64) -> Vec<Lease>;
    #[must_use] pub fn is_denied(&self, id: &PassportId) -> bool;
    /// Deny `id`: refuse it from now on and end its leases. Returns how many
    /// leases ended.
    pub fn deny(&self, id: &PassportId, now_secs: u64) -> anyhow::Result<usize>;
    /// Write `lease` if the passport is not denied and `limit` allows it.
    pub(crate) fn grant(&self, lease: Lease, limit: Option<u64>, now_secs: u64) -> Result<(), RegisterError>;
}
```

`open` validates `db_name` exactly like `AlertStore::open` (no `/`, `\`,
`..`), sets `PRAGMA journal_mode=WAL;`, runs `init_schema`, then
`DELETE FROM passport_leases WHERE lease_ends_at <= ?now`, then loads both
tables into the `Cache`. A stored `passport_id` that does not parse as a
`PassportId`, or a `limits` value that is not a JSON object, is an error
(`open` fails); the store only writes valid values.

`grant` pseudo-code:

```text
conn = self.inner.conn.lock()                 // one writer at a time
{
    cache = self.inner.cache.read()
    if cache.is_denied(&lease.passport_id): return Err(Denied)
    if let Some(limit) = limit:
        if cache.other_running_subjects(&lease.passport_id, &lease.subject, now) >= limit:
            return Err(LimitReached { limit })
}
ends = i64::try_from(lease.lease_ends_at).map_err(store_err)?
now_i = i64::try_from(now).map_err(store_err)?
tx = conn.transaction()
tx.execute("DELETE FROM passport_leases WHERE lease_ends_at <= ?1", [now_i])
tx.execute("INSERT INTO passport_leases
              (passport_id, owner_did, subject, lease_ends_at, registered_at, limits)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT (passport_id, subject) DO UPDATE
            SET owner_did = excluded.owner_did, lease_ends_at = excluded.lease_ends_at,
                registered_at = excluded.registered_at, limits = excluded.limits", ...)
            // ?6 = serde_json::to_string(&lease.limits)
tx.commit()
cache = self.inner.cache.write(); cache.prune(now); cache.insert(lease)
Ok(())
// every rusqlite error -> RegisterError::Store(e.to_string())
```

A renewal replaces the lease end with the new value, even when it is earlier
(the newest registration wins).

`deny` pseudo-code:

```text
conn = lock; tx = conn.transaction()
tx.execute("INSERT OR IGNORE INTO passport_denied (passport_id, denied_at) VALUES (?1, ?2)")
removed = tx.execute("DELETE FROM passport_leases WHERE passport_id = ?1")
tx.commit()
cache = write; cache.deny(id); cache.remove_passport(id)
Ok(removed)
```

Tests (`lease_store/tests.rs`; use `tempfile::tempdir()` for the file
tests):

- `leases_survive_a_restart` (grant with limits `{"max_bytes": 5}`, drop the
  store, `open` the same file: `is_admitted` is true, and `lease_for`
  returns the same `Lease`, limits included).
- `lease_for_picks_the_newest_registration`: subject S, passport P1 at
  `registered_at = now`, then P2 at `now + 10` → P2's lease. Renew P1 at
  `now + 20` → P1's lease. Deny P1 → P2's lease. Equal `registered_at` →
  the larger `lease_ends_at`.
- `lease_for_ignores_an_ended_lease`.
- `an_ended_lease_does_not_admit` (`is_admitted` at `lease_ends_at` is false;
  at `lease_ends_at - 1` true).
- `ended_leases_are_deleted_at_open`.
- `deny_ends_leases_and_survives_a_restart` (grant two subjects, deny,
  both not admitted, reopen, still denied, `grant` → `Denied`).
- `deny_in_advance_refuses_a_later_grant`.
- `the_limit_counts_distinct_running_subjects` (limit 2: A, B ok; C →
  `LimitReached`; A renews → ok; after B's lease ends, C → ok).
- `a_renewal_replaces_the_lease_end`.
- `running_leases_lists_only_running_ones_in_order`.
- `a_subject_with_two_passports_stays_admitted_when_one_is_denied`.
- `open_refuses_a_path_in_the_db_name`.

### Step 11 — `Registrar` (`src/registrar.rs`)

```rust
/// The registration checks of one service for one use.
#[derive(Debug)]
pub struct Registrar {
    use_: PassportUse,
    host_node_did: String,
    accepted_roots: Vec<String>,
    challenges: ChallengeStore,
    leases: LeaseStore,
}

impl Registrar {
    /// `accepted_roots`: the operator roots this service trusts (the host's
    /// controller DID and/or its node DID). It must not be empty.
    pub fn new(use_: PassportUse, host_node_did: String, accepted_roots: Vec<String>, leases: LeaseStore)
        -> anyhow::Result<Self>;
    #[must_use] pub fn passport_use(&self) -> &PassportUse;
    #[must_use] pub fn leases(&self) -> &LeaseStore;
    pub fn issue_challenge(&self, now_secs: u64) -> anyhow::Result<Challenge>;
    /// Every offline check: shape, proof of possession, signer binding,
    /// chain, and the limit value. Writes nothing, except that a challenge
    /// is used up. The caller then runs the DHT revocation check on
    /// `checked.token()` (it needs the network) and, if that passes, calls
    /// `grant`.
    pub fn check(&self, token: &CapabilityToken, proof: &Proof, binding: &SignerBinding, now_secs: u64)
        -> Result<CheckedRegistration, RegisterError>;
    /// The deny check, the use limit count and the lease write, under one
    /// lock. Call it only after the revocation check passed.
    pub fn grant(&self, checked: CheckedRegistration, now_secs: u64) -> Result<Lease, RegisterError>;
}

/// The result of `Registrar::check`. Only `check` can make one (private
/// fields), so `grant` never writes a lease for a token that was not checked.
#[derive(Debug, Clone)]
pub struct CheckedRegistration {
    token: CapabilityToken,   // the presented child token, passport inside
    lease: Lease,             // the lease `grant` will write
    limit: Option<u64>,       // the use limit read from the passport
}

impl CheckedRegistration {
    /// The checked token, for the revocation check of every chain edge.
    #[must_use] pub fn token(&self) -> &CapabilityToken;
    /// The lease `grant` will write, for logs.
    #[must_use] pub fn lease(&self) -> &Lease;
}
```

`grant` returns the written `Lease`. The lease already holds the passport
limits (D-B1-16), so no separate result type is needed.

How a service uses the two calls (B2 and C8 write this; shown here so the
split is clear):

```text
let checked = registrar.check(&token, &proof, &binding, now_secs())?;
if !ucan_chain_not_revoked(checked.token(), resolver).await {
    return Err(RegisterError::Revoked);
}
let lease = registrar.grant(checked, now_secs())?;
```

`check` is the change doc's steps 1 to 3 and the reading part of step 4,
after a shape check. `grant` is the rest of step 4 and step 5. Keep each
step a small private function so `check` reads as a list:

```text
check(token, proof, binding, now):
    let chain = PassportChain::parse(token)?              // shape (D-B1-1)
    let proven = proof.verify(&self.challenges, &self.host_node_did,
                              &self.use_, token, now)?    // step 1: proof
    binding.check(proven, chain.subject(), now)?          // step 2: binding
    self.check_chain(&chain, now)?                        // step 3: chain
    let limit = chain.count_limit(&self.use_)?            // step 4a: limit from passport only
    let passport_id = PassportId::of(chain.passport).map_err(malformed)?
    let lease = Lease {
        passport_id, owner_did: chain.passport.audience_did.clone(),
        subject: chain.subject().to_string(),
        lease_ends_at: min(chain.leaf.expires_at_secs,
                           chain.passport.expires_at_secs,
                           now.saturating_add(self.use_.max_lease_secs)),
        registered_at: now,
        limits: chain.passport.facts.clone(),             // never the child's facts
    }
    Ok(CheckedRegistration { token: token.clone(), lease, limit })

grant(checked, now):
    // The revocation lookup may take seconds. A token that expired in that
    // time is refused, so a lease never starts already ended.
    if now >= checked.lease.lease_ends_at: return Err(Expired)
    self.leases.grant(checked.lease.clone(), checked.limit, now)?   // deny, count, write
    Ok(checked.lease)
```

`grant` keeps the `registered_at` that `check` set. The lease end was fixed
by `check` from the token times, so a slow lookup does not move it.

`PassportChain` (crate-private):

```rust
struct PassportChain<'a> { leaf: &'a CapabilityToken, passport: &'a CapabilityToken }

impl<'a> PassportChain<'a> {
    fn parse(leaf: &'a CapabilityToken) -> Result<Self, RegisterError> {
        // Malformed unless: leaf.proofs.len() == 1, passport.proofs.is_empty(),
        // leaf.capabilities.len() == 1, passport.capabilities.len() == 1,
        // and leaf.audience_did is canonical (ensure_canonical_did, D-B1-20).
        // Each failure has its own message ("the child token must carry
        // exactly one proof, the passport", ...). No signature work here.
    }
    fn subject(&self) -> &str { &self.leaf.audience_did }
    fn count_limit(&self, use_: &PassportUse) -> Result<Option<u64>, RegisterError> {
        match self.passport.facts.get(use_.count_key) {
            None => Ok(use_.open_count_limit),
            Some(v) => v.as_u64().map(Some).ok_or_else(||
                Malformed(format!("{} is not a whole number", use_.count_key))),
        }
    }
}
```

`check_chain` pseudo-code:

```text
if !accepted_roots.contains(passport.issuer_did): Err(UntrustedRoot)
if passport.audience_did != leaf.issuer_did:       Err(BrokenChain)
for t in [passport, leaf]:
    if !use_.names_exactly(&t.capabilities[0], host_node_did):
        Err(WrongCapability { ability: use_.ability, resource: use_.resource(host).0 })
// D-B1-19: checked here, not left to verify_chain. When the owner is also an
// accepted root, verify_chain roots the child token on its own and never
// reads the passport's caveat.
if !passport.capabilities[0].can_delegate():
    Err(InvalidChain("the passport forbids delegation"))
if now >= passport.expires_at_secs || now >= leaf.expires_at_secs: Err(Expired)
let is_root = |iss: &str, cap: &Capability|
    accepted_roots.iter().any(|r| r == iss) && use_.names_exactly(cap, host_node_did);
let opts = ChainVerifyOpts { expected_audience_did: &leaf.audience_did,
                             is_trusted_root: &is_root, now_secs: now };
// verify_chain checks every signature (the passport's too, also when the
// leaf is rooted on its own), the not-before times and the anchor rule.
let granted = verify_chain(leaf, &opts).map_err(|e| InvalidChain(e.to_string()))?;
// verify_chain drops a capability it cannot back, so an empty result is a
// refusal.
if !granted.iter().any(|c| use_.names_exactly(c, host_node_did)):
    Err(InvalidChain("the passport does not back the child token's capability"))
Ok(())
```

Why the explicit checks come before `verify_chain`: `verify_chain` returns
an empty capability list (not an error) for an untrusted root or a broken
link, so it cannot tell the cases apart. The change doc asks for one error
for each case.

Tests: one shared fixture in `registrar/tests.rs`, and the cases in
`registrar/tests/<topic>.rs` so no file passes 800 lines.

Fixture (in `registrar/tests.rs`):

```rust
struct Fixture {
    operator: Identity,     // the accepted root
    host_node_did: String,  // a fresh did:key; not the operator
    owner: Identity,
    subject: Identity,      // relay: node key; mailbox: member master
    instance: Identity,     // mailbox: the instance key
    registrar: Registrar,
    now: u64,               // wall-clock now, read ONCE; every token and every check uses it
}
impl Fixture {
    fn relay() -> Self;                                       // RELAY_HOME
    fn mailbox() -> Self;                                     // MAILBOX_USE
    fn passport(&self, limits: Map<String, Value>, life_secs: u64) -> CapabilityToken;
        // issue_passport(.., self.now)
    fn child(&self, passport: &CapabilityToken, subject: &Identity) -> CapabilityToken;
        // child_token(.., self.now)
    fn challenge_proof(&self, signer: &Identity, token: &CapabilityToken, at: u64) -> Proof;
        // registrar.issue_challenge(at) + sign_challenge
    fn transport_proof(did: &str) -> Proof;
    fn instance_binding(&self, master: &Identity, instance: &Identity) -> SignerBinding;
        // DelegationCertificate::issue(master, instance.public_key(), CERT_LIFE_SECS, SCOPE_SERVICE_INSTANCE)
    fn register_relay(&self, token, signer: &Identity, at: u64) -> Result<Lease, RegisterError>;
        // challenge form + SameKey: issue the challenge at `at`, then
        // check(.., at) and grant(.., at)
    fn register_mailbox(&self, token, instance: &Identity, master: &Identity, at: u64)
        -> Result<Lease, RegisterError>;
        // transport form + InstanceCertificate: check(.., at), grant(.., at)
}
```

Every helper takes the registration time `at`, and `register_relay` issues
its challenge at that same `at`. So a test that registers late (at
`now + life`, or at `now + CERT_LIFE_SECS + 5`) gets the error it tests, not
`ChallengeExpired`: the challenge is always fresh. Only the challenge-life
tests issue a challenge at one time and register at another, and they do it
by hand. Most tests pass `at = f.now`.

The clock rule for these tests (D-B1-11): tokens and challenges come only
from `Fixture::now`, so boundary tests compare exact values. The
`DelegationCertificate` is the exception: `DelegationCertificate::issue`
reads the wall clock, and its signed bytes are private to `crates/identity`,
so a test cannot set its times. Its wall-clock `issued_at_secs` may be a
second or two after `Fixture::now`. That is safe for the valid cases:
`verify_chain_at` allows an `issued_at` up to 300 seconds in the future. For
the expired case, verify at `self.now + CERT_LIFE_SECS + 5`, never at the
exact boundary.

`registrar/tests/happy.rs`:

- `relay_registration_writes_a_lease` (challenge + same key; subject is
  admitted; `lease.limits` equals the passport facts; `lease_for(subject)`
  returns the same lease; `registered_at == now`).
- `a_self_hosted_relay_where_the_owner_is_a_root_registers`: roots =
  [controller], and the controller is also the owner (it receives the
  passport and signs the child token) → Ok.
- `mailbox_registration_writes_a_lease_for_the_master` (transport +
  instance certificate; the lease subject is the master DID, not the instance
  DID).
- `challenge_form_works_with_an_instance_certificate` (cross combination).
- `transport_form_works_with_the_same_key` (cross combination).
- `a_passport_rooted_at_either_accepted_root_is_accepted` (roots = [controller,
  node]; one passport from each).
- `the_lease_ends_at_the_earliest_limit` (three cases: child expiry first,
  passport expiry first, `max_lease_secs` first).
- `renewing_moves_the_lease_end`.

`registrar/tests/refusals.rs` (one test per `RegisterError` variant; run each
of the chain cases with **both** fixture helpers, so both proof forms are
covered):

- `malformed_chain_shapes_are_refused` (no proof; two proofs; a passport with
  its own proof (three tokens); two capabilities; `max_nodes: "3"`).
- `a_passport_from_an_untrusted_root_is_refused`.
- `a_child_from_someone_other_than_the_passport_audience_is_refused`
  (`BrokenChain`).
- `a_wrong_ability_or_resource_is_refused`: the other use's ability; bare
  `substrate:<host>`; `substrate:<other host>/relay`; `substrate/admin` on the
  bare resource (all `WrongCapability`).
- `an_expired_passport_or_child_is_refused` (helpers with `at = now + life`
  → `Expired`; with `at = now + life - 1` → Ok. Exact, because every token
  time comes from `Fixture::now`, and the helper's challenge is fresh at
  `at`. Use a `life` above 60 seconds, for example 3,600, so the test would
  catch a helper that issued its challenge at `now`).
- `a_passport_that_forbids_delegation_is_refused`: `can_delegate: false`
  caveat on the passport (built by hand and signed with step 2's `sign`) →
  `InvalidChain`. Run it twice: with roots = [operator], and with the owner
  also in the roots. The second run is the case where `verify_chain` alone
  would accept it (D-B1-19).
- `a_tampered_passport_is_refused_when_the_owner_is_a_root`: the owner is in
  the roots; change `facts` of the passport after signing → `InvalidChain`
  (`verify_chain` still checks the proof's signature).
- `a_non_canonical_subject_is_refused`: a child token whose `audience_did` is
  another spelling of the subject's key (found as in step 6), built by hand
  → `Malformed`. Skip with a clear assert message if step 6's search finds
  no other spelling.
- `a_denied_passport_is_refused`.
- `the_use_limit_is_enforced` (`max_nodes: 1`; second subject →
  `LimitReached { limit: 1 }`; the first subject renews → Ok).
- `limits_in_the_child_token_are_ignored`: build the child by hand with
  `facts: {"max_nodes": 1000}` under a passport with `max_nodes: 1`, sign it
  with step 2's `sign`; second subject still → `LimitReached { limit: 1 }`.

`registrar/tests/possession.rs`:

- `a_leaked_passport_and_child_token_are_not_enough`: an attacker holds both
  tokens but not the subject key.
  - challenge form, signed by the attacker, `signer_did = subject` →
    `BadSignature`;
  - challenge form, signed by the attacker, `signer_did = attacker` →
    `NotBound`;
  - transport form, `verified_did = attacker`, same key → `NotBound`;
  - transport form, attacker's instance with a certificate from the
    attacker's own master → `NotBound`.
- `an_instance_certificate_must_match_master_key_and_scope`: wrong
  `master_did`, wrong `temporary_did`, scope `routing` (the router accepts
  it, the core must not, D-B1-17), expired certificate — each `NotBound`.
  For the expired case, make the passport and child live longer than
  `CERT_LIFE_SECS + 5`, and register at `now + CERT_LIFE_SECS + 5` (see the
  clock rule above).
- `a_challenge_signature_for_another_service_is_refused`: sign with
  `service_did` of another host → `BadSignature`.
- `a_challenge_signature_for_another_token_is_refused`: sign over child A,
  present child B → `BadSignature`.
- `a_challenge_signature_for_the_other_use_is_refused`: sign with
  `MAILBOX_USE`, present to the relay registrar → `BadSignature`.
- `a_challenge_is_single_use_even_after_a_bad_signature`: bad signature,
  then a good signature on the same challenge → `UnknownChallenge`.
- `an_old_challenge_is_refused` (by hand: issue the challenge at `now`, call
  `check` at `now + 60` → `ChallengeExpired`).
- `check_writes_nothing`: `check` succeeds, then `grant` is not called →
  the subject is not admitted, and `running_leases` is empty. (A service
  that refuses with `Revoked` leaves no lease behind.)
- `grant_refuses_a_lease_that_ended_during_the_lookup`: `check` at `now`
  with a child that ends at `now + 10`; `grant` at `now + 10` → `Expired`.
- `a_deny_between_check_and_grant_wins`: `check` → Ok; `deny` the passport;
  `grant` → `Denied`.
- `an_unknown_challenge_is_refused`.
- `a_swapped_passport_under_the_same_child_is_refused`: the owner holds two
  passports; sign over child-with-passport-1, present the same child body
  with passport 2 → `BadSignature` (this is the reason for D-B1-4).

### Step 12 — `RequestLimiter` (`src/limiter.rs`)

```rust
pub const REQUESTS_PER_MINUTE: u32 = 10;
const WINDOW_SECS: u64 = 60;
const MAX_TRACKED_SOURCES: usize = 10_000;

#[derive(Debug)]
pub struct RequestLimiter { per_window: u32, max_sources: usize, windows: Mutex<HashMap<IpAddr, Window>> }

#[derive(Debug, Clone, Copy)]
struct Window { started_at: u64, count: u32 }

impl Default for RequestLimiter { /* REQUESTS_PER_MINUTE, MAX_TRACKED_SOURCES */ }

impl RequestLimiter {
    /// Count one request from `source`. False when the source has used its
    /// requests for this minute, or when too many sources are tracked.
    pub fn allow(&self, source: IpAddr, now_secs: u64) -> bool;
}
```

Pseudo-code:

```text
allow(source, now):
    w = lock
    if let Some(win) = w.get_mut(&source) && now < win.started_at + WINDOW:
        if win.count >= per_window { return false }
        win.count += 1; return true
    if !w.contains_key(&source) && w.len() >= max_sources:
        w.retain(|_, win| now < win.started_at + WINDOW)
        if w.len() >= max_sources { return false }
    w.insert(source, Window { started_at: now, count: 1 }); true
```

Tests: 10 allowed then the 11th refused; a new minute resets; two addresses
are independent; with `max_sources = 2` (a `#[cfg(test)]` constructor), a
third address is refused until a window ends.

### Step 13 — wire types (`src/wire.rs`)

These are the bodies of `POST /v1/home-relay/challenge` and `POST
/v1/home-relay/register` (B2 serves them, B3 calls them). Defining them here
lets the size-and-shape rule be tested in the core.

```rust
/// Largest accepted registration body.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChallengeResponse { pub challenge: String, pub expires_in_secs: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    /// The child token, with the passport as its one proof.
    pub token: CapabilityToken,
    pub challenge: String,     // 64 hex chars
    pub signer_did: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterResponse { pub passport_id: PassportId, pub lease_ends_at: u64 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody { pub code: String, pub message: String }

impl RegisterRequest {
    /// Size first, then JSON shape; no signature is checked here.
    pub fn from_slice(body: &[u8]) -> Result<Self, RegisterError> {
        if body.len() > MAX_REQUEST_BYTES { return Err(RequestTooLarge { max: MAX_REQUEST_BYTES }) }
        serde_json::from_slice(body).map_err(|e| Malformed(e.to_string()))
    }
    pub fn into_parts(self) -> Result<(CapabilityToken, Proof), RegisterError> {
        let challenge = Challenge::from_hex(&self.challenge)?;
        Ok((self.token, Proof::Challenge { challenge, signer_did: self.signer_did, signature: self.signature }))
    }
}

impl From<&RegisterError> for ErrorBody { /* code(), to_string() */ }
```

Tests (`wire/tests.rs`): a body of `MAX_REQUEST_BYTES + 1` bytes of valid
JSON padding (e.g. spaces) → `RequestTooLarge` (shows the size check runs
before the parse); invalid JSON → `Malformed`; an unknown field → `Malformed`;
a bad challenge hex → `Malformed`; a full round trip (fixture → request →
`from_slice` → `into_parts` → `Registrar::check` → `Registrar::grant` → Ok).

### Step 14 — `roymctl coordinator passport issue`

`apps/roymctl/Cargo.toml`: add `syneroym-passport.workspace = true`.

`apps/roymctl/src/commands/identity.rs`: make `require_key_present` and
`identity_key_path` `pub(crate)` (reuse, do not copy).

New file `apps/roymctl/src/commands/coordinator.rs`:

```rust
//! Coordinator operator subcommands: passports for a private relay.

#[derive(Subcommand, Debug, Clone)]
pub enum CoordinatorCommands {
    /// Operator passports for a relay with `access = "passport"`
    Passport {
        #[command(subcommand)]
        command: PassportCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PassportCommands {
    /// Issue a passport that lets one owner's substrates use this relay as
    /// their home relay. Offline: it only signs. Give the file to the owner.
    ///
    /// A relay cannot use passports yet: a coordinator with
    /// `access = "passport"` still refuses to start. Passports issued now
    /// will work once relays support them.
    #[command(group(ArgGroup::new("signer").required(true).args(["from", "node_key"])))]
    Issue {
        /// Local identity that signs: the controller of the relay's host
        /// substrate.
        #[arg(long)]
        from: Option<String>,
        /// Sign with the node key `<dir>/substrate.key`. Only for a host
        /// substrate that has no controller.
        #[arg(long)]
        node_key: bool,
        /// The owner DID the passport is for.
        #[arg(long)]
        to: String,
        /// Days until the passport expires.
        #[arg(long)]
        expires_days: u64,
        /// Most distinct substrates that may hold a lease at one time.
        /// Omitted: no limit.
        #[arg(long)]
        max_nodes: Option<u64>,
        /// Where to write the passport. Omitted: standard output.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub fn handle(command: &CoordinatorCommands, substrate_opt: Option<String>, dir: &Path) -> anyhow::Result<()>;
```

Change the help line "A relay cannot use passports yet ..." in B2, when the
coordinator accepts `access = "passport"`. Record this as a B2 task in the
passport change doc (step 15).

`handle_issue` pseudo-code (keep it under 100 lines; split `load_signer`):

```text
host_node_did = super::get_substrate_did(substrate_opt, dir)?   // existing private helper in
                                                                 // commands.rs; a child module may call it
signer = match (from, node_key):                                 // clap guarantees exactly one
    (Some(name), false) => Identity::load_from_path(identity::require_key_present(dir, name, "Identity")?)?
    (None, true) => {
        let key = Identity::load_from_path(dir.join(DEFAULT_SUBSTRATE_KEY_FILE))
            .context("--node-key given, but there is no substrate.key in --dir")?;
        ensure derive_did_key(key) == host_node_did,
            else "substrate.key is the key of <did>, not of --substrate <host>"
        key
    }
    _ => bail!("pass exactly one of --from <name> or --node-key")
mut limits = Map::new()
if let Some(n) = max_nodes { limits.insert(facts::MAX_NODES, json!(n)) }
ensure expires_days > 0
passport = issue_passport(&signer, &host_node_did, &RELAY_HOME, &to,
                          limits, expires_days.checked_mul(86_400)?,
                          syneroym_passport::now_secs())?   // step 4 helper
json = serde_json::to_string_pretty(&passport)?
match out:
    Some(path) => write_secret_file(&path, json.as_bytes(), "passport")?   // existing helper, mode 0600
    None => println!("{json}")
eprintln!("passport id: {}", PassportId::of(&passport)?)
// The operator needs the id to deny the passport later.
```

The mailbox variant (C8) will be a second `Issue`-like arm that calls the
same `issue_passport` with `MAILBOX_USE` and its own limit flags. Keep the
shared part (`load_signer`, writing the file, printing the id) in small
functions so C8 only adds the flag parsing.

`apps/roymctl/src/commands.rs`:

- `pub mod coordinator;`
- In `Commands`, after `Registry`:

  ```rust
  /// Operate a coordinator (relay) you run
  Coordinator {
      #[command(subcommand)]
      command: coordinator::CoordinatorCommands,
  },
  ```

- In `run`: `Commands::Coordinator { command } => coordinator::handle(&command, substrate_opt, &dir)?,`

Tests:

- `apps/roymctl/tests/cli_args.rs`:
  - `test_coordinator_passport_issue_help` (help lists `--from`,
    `--node-key`, `--to`, `--expires-days`, `--max-nodes`, `--out`, and the
    "cannot use passports yet" line).
  - `test_coordinator_passport_issue_needs_a_signer` (no `--from` and no
    `--node-key` → clap error; both → clap error).
- `apps/roymctl/src/commands/coordinator/tests.rs` (in-tree, calls
  `handle_issue` directly with a `tempdir`):
  - `issue_from_a_controller_writes_a_passport_that_registers`: create a
    local identity, issue to an owner DID with `--max-nodes 2`, read the
    file, build a child token with `syneroym_passport::child_token`,
    run `check` and then `grant` on it at a `Registrar` (`RELAY_HOME`,
    roots = [controller DID], in-memory store) → Ok, and `lease.limits == {"max_nodes": 2}`; the file
    mode is `0o600` on Unix.
  - `issue_with_the_node_key` (write a `substrate.key`, `--node-key`).
  - `issue_refuses_a_node_key_that_is_not_the_substrate`.
  - `issue_refuses_zero_expiry_days`.

### Step 15 — docs and close-out of the slice

- `docs/planning/changes/operator-passport/change.md`:
  - `## Deviations`: write `D-B1-1` to `D-B1-22` (short form of §2), and any
    difference you made from this plan. Mark D-B1-15 (the service runs the
    revocation check), D-B1-18 (deny and list through the running service)
    and the `issue` help line as **requirements on B2**, and D-B1-17 (the
    certificate in the request body) as a **requirement on C4 and C8**.
  - `## Tasks`: tick "Core module ...". Leave `status: approved` (the change
    closes at B3).
  - Design, "roymctl": change the proposed `issue --audience <did>
    --expires-in <duration>` to `issue (--from <name> | --node-key) --to <did>
    --expires-days <n>` (if the owner confirms D-B1-14 and D-B1-21).
  - Acceptance table, `Test` column: fill `P-1` (`issue/tests.rs`,
    `coordinator/tests.rs` in `roymctl`), `P-2` and `P-3`
    (`registrar/tests/refusals.rs`, `registrar/tests/possession.rs`), `P-7`
    (`challenge_store/tests.rs`, `limiter/tests.rs`, `wire/tests.rs`), `P-9`
    (`registrar/tests/possession.rs`), and `P-8` for its `issue` part only
    (write "issue: `coordinator/tests.rs`; deny and leases: B2").
- `docs/planning/changes/service-mailbox/change.md`, "Dedicated mailbox and
  passport": add D-B1-17 in one or two sentences: the `register` request
  carries the instance certificate, because the router does not pass it on.
  The passport core checks its `service-instance` scope. Also add one
  sentence: the mailbox reads its raised quotas from `LeaseStore::lease_for`,
  and with two passports the newest registration wins (D-B1-16).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/status.md`:
  add a B1 row (Done, evidence: test files and counts, `mise run verify`),
  and change the last row to "B2, B3, C2 to C8".
- `task.md`: the status line at the top says "S0 and A1 to A4 are done";
  make it "S0, A1 to A4, C1 and B1 are done". In §3 B1, add "Done <date>.
  Decisions `D-B1-1` to `D-B1-22` are in the passport change doc." In §3
  B2, add: "Run the DHT revocation check between `Registrar::check` and
  `Registrar::grant` (D-B1-15);
  `deny` and `leases` go through the running service (D-B1-18); remove the
  'cannot use passports yet' line from the `issue` help." In §3 C8, add:
  "`register` carries the instance certificate (D-B1-17); quotas come from
  `LeaseStore::lease_for` (D-B1-16); the DHT revocation check runs between
  `Registrar::check` and `Registrar::grant` (D-B1-15)."
- `docs/planning/deferred-backlog.md`, §3 (access control):
  - "Passport request limiter keys on the full IPv6 address" — an attacker
    with a `/64` gets many limits. Group IPv6 by `/64`. Target TBD. Link
    `crates/passport/src/limiter.rs`.
  - "No undeny for a passport" — a deny is permanent; the operator must
    issue a new passport. TBD. Link `lease_store.rs`.
  - "A deny or a revocation does not end a running lease at once" — the
    lease runs to its end (at most 24 hours for the relay, 30 days for the
    mailbox), because the relay cannot drop a connected client and the
    revocation check runs only at registration. TBD. Link the change doc.
  - Only if the owner rejects D-B1-15: "An owner cannot revoke one child
    token" — only the operator can deny the whole passport. TBD.
- Living docs: no change in B1. No running service uses the crate, and the
  change doc's close-out list (architecture config table, requirements Relay
  section, TERMINOLOGY "passport", "child token", "lease", operator note in
  the developer guide) belongs to B2/B3, when the feature can be used. State
  this in the PR text.

## 4. Things that are ambiguous or stale in task.md and the change docs

1. **Stale: task.md status line and serial order.** The top line says "S0
   and A1 to A4 are done"; C1 is also done (status.md). §4 gives the serial
   order "S0, A1, C1, A2, A3, B1, A4, B2, ..."; the owner chose on 2026-10-13
   "A4, C1, B1, B2, B3, C2, ..." (task.md does not record it). Fix both lines
   in step 15.
2. **Stale: "B1 can run in parallel with A1 to A3".** True for the code, but
   the owner chose strictly serial work. No action.
3. **Owner check — chain shape (D-B1-1).** "The chain has two tokens" could be
   a description, not a rule. Should the service also accept a single passport
   issued straight to the subject (operator = owner)? This plan says no: one
   shape, one code path, and the owner always signs the child token with
   `roymctl`.
4. **Owner check — "a hash of the token" (P-9, D-B1-4).** The signed body of a
   `CapabilityToken` does not cover `proofs`. This plan hashes the whole
   presented token, so a signature also fixes which passport is under the
   child. The change doc's sentence stays true; the deviation says which hash.
5. **Owner check — what `max_nodes` counts (D-B1-5).** The change doc says
   "how many distinct nodes use one passport". This plan counts nodes with a
   running lease (a node that stops renewing frees its place). The other
   reading, "every node ever registered", would need a second table and
   would lock a passport for good after a node key changes.
6. **Owner check — `roymctl` flag names (D-B1-14, D-B1-21).** The change
   doc marks the `roymctl` names as a proposal. It differs from the existing
   CLI in two ways:
   - *Signer and receiver.* The change doc proposes `--audience` and no
     signer flag. The existing token command `identity issue-grant` uses
     `--from` (signer) and `--to` (receiver). This plan follows `issue-grant`:
     `--to <owner did>`, and a required choice of `--from <name>` or
     `--node-key`.
   - *Expiry unit.* The change doc proposes `--expires-in <duration>`.
     `roymctl` has no duration parser and uses two units today:
     `--expires-days` (`identity issue-grant`) and `--expires-hours`
     (`identity certify-instance`, `certify-signing`, `roym transaction`).
     This plan uses `--expires-days`, because a passport lives weeks or
     months.
   The host node DID comes from the existing global `--substrate` flag (or
   `<dir>/substrate.key`). `--out` is new and optional. The help text says
   that a relay cannot use passports until relays support
   `access = "passport"`.
7. **Owner check — accepted roots (now needed in B1, because of
   `--node-key`).** The change doc says "The service accepts a passport
   rooted at either one" (controller or node DID). ADR-0015 A10.4 says "the
   controller ..., or the host node DID **when the substrate has no
   controller**". These differ when a controller exists: may the node key
   still issue passports? The core takes a list (`accepted_roots`), so the
   core does not need the answer. But a `roymctl issue` default would decide
   it, so B1 has no default signer (D-B1-21). **Recommendation: follow the
   ADR.** The controller is the owner of the node; a node key that can mint
   passports beside a controller is a second, silent authority. B2 then
   passes `accepted_roots = [controller]` when a controller exists, else
   `[node DID]`, and fixes the change doc's sentence. The `--node-key` help
   text in B1 already says "only for a host substrate that has no
   controller".
8. **For B2 — subject DID versus Iroh endpoint id.** The lease subject for the
   relay is the node DID (`did:key:h<z-base-32>`). The Iroh relay access
   function receives an `EndpointId`. They are the same Ed25519 key: the
   substrate builds its Iroh endpoint from the bytes of its node key
   (`crates/substrate/src/identity.rs:99`, `get_secret`, passed to
   `build_iroh_endpoint` in `crates/router/src/connection_router.rs:93`). B2
   converts `EndpointId` → node DID (`derive_did_key` over the public key
   bytes) before calling `LeaseStore::is_admitted`. The static `access` list
   holds endpoint ids, not DIDs (`crates/core/src/config/access.rs`).
9. **For B2 — `max_lease_secs` "default".** The change doc says "relay
   default 24 hours", which suggests a setting. B1 gives
   `PassportUse::with_max_lease_secs`; B2 decides whether a config field
   exists.
10. **For C8 — open limit of `max_services`.** ADR-0015 A10 says "a missing
    key means the open limit of the service applies". The mailbox doc gives no
    open number for `max_services` (it says a per-owner queue limit "cannot
    be enforced" for open queues). B1 sets `open_count_limit: None` for both
    uses. C8 decides.
11. **Owner check — DHT revocation (D-B1-15).** ADR-0015 §2 and A7 require
    the DHT revocation check on every token in the chain. The first version
    of this plan skipped it, which broke the ADR without asking. The plan
    now keeps the ADR rule, but the check runs in the service (B2, C8), with
    the router's existing `ucan_chain_not_revoked`, between
    `Registrar::check` and `Registrar::grant` (all offline checks first, so
    the sender cannot cause network lookups with an invalid request). B1
    adds the `Revoked` error and the two-call split. This also
    closes the gap "an owner cannot cut one node": the owner lists the node
    key (or member master) in the `revoked_keys` of their master anchor, and
    the `(owner, subject)` edge is refused at the next registration. The
    other choice is to skip revocation for passports and rely on the deny
    list. That needs an ADR-0015 amendment, and the owner could then cut one
    node only by asking the operator to deny the whole passport.
    **Recommendation: keep the ADR rule, as planned.**
12. **`P-4` and `P-5` and `P-6` at the store level only.** B1 tests them on
    `LeaseStore`. The full criteria ("refused when it connects the next time",
    "after the operator's service restarts") need the relay, so their
    `Test` column in the change doc stays empty until B2/B3 add the
    integration tests. B1 fills the `Test` column for `P-1`, `P-2`, `P-3`,
    `P-7` (core parts), `P-9`, and the `issue` part of `P-8` (step 15).
13. **`P-7`: "size and shape checks before any signature check".** In B1 the
    size check is `RegisterRequest::from_slice`, and the shape check is
    `PassportChain::parse`, both before any signature work. B2 must also set
    axum's `DefaultBodyLimit::max(MAX_REQUEST_BYTES)` on the two routes so a
    large body is never read into memory, and must call
    `RequestLimiter::allow` before reading the body.

## 5. Completion steps

1. `cargo +nightly-2026-04-06 fmt --all`.
2. `cargo clippy -q --workspace --all-targets --all-features -- -D warnings`.
3. `cargo nextest run -p syneroym-passport -p syneroym-ucan -p roymctl`.
4. Import clean-up pass over every edited file (types by `use`, functions by
   their parent module, no inline paths with many `::`).
5. `cargo dupes report` filtered to the changed files (recipe in
   `AGENTS.md`). The lock-and-map-error lines in `lease_store.rs` are the
   most likely duplicates.
6. Deferred backlog and change-doc updates of step 15.
7. `mise run verify` in the background; every gate must pass, with no
   `--skip`. If the duplication gate reports a value better than the
   baseline, lower `MAX_EXACT_DUPLICATE_PERCENT` as the gate message says.
8. Commit on `feat/m06d-b1-implementation` (Conventional Commits, for example
   `feat(passport): add passport core crate`), open one PR, with the CLA line
   in the body.
