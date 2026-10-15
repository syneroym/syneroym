---
status: approved
living-docs-touched:
---
# Operator passport

*Reader: developers and operators.*

## Problem

Some infrastructure services are not open to everyone. An operator runs a private or paid Iroh relay, or a dedicated mailbox. Today the only gate is a fixed list in the config (`[roles.coordinator] access` lists Iroh endpoint ids). The list is read once at startup. To admit a new user the operator must edit the config and restart.

This change adds a **passport**: a signed, time-limited token. The operator gives it to a user outside the system (email, chat). The user's owner key re-delegates it to the key of their own substrate or service, with `roymctl`. The substrate shows the result to the operator's service and receives a **lease**: access for a limited time, which it renews.

The passport is one mechanism with two users:

| Use | Operator | Passport audience | Subject (child token audience) | Proof of possession | Specified in |
| --- | --- | --- | --- | --- | --- |
| Home relay (this doc) | Controller of the coordinator's host substrate | Substrate owner DID | The substrate node key | The node key signs a challenge, over HTTPS | [Use: home relay](#use-home-relay) |
| Dedicated mailbox | Controller of the mailbox's host substrate | Service owner DID | The member master DID | The transport identity: the instance key, checked by the Iroh connection and bound to the master by its instance certificate | [service-mailbox](../service-mailbox/change.md) |

This doc specifies the shared core and the home-relay use. The mailbox doc specifies the mailbox use. It adds only its own ability, limits and quotas.

Open services do not need a passport. Open relays and open mailboxes are listed by a community registry ([registry-listed-coordinators](../registry-listed-coordinators/change.md), [service-mailbox](../service-mailbox/change.md)). A service that needs a passport is never listed. The user learns about it directly and configures it by hand.

## Scope and non-goals

- In scope:
  - **Core:** the token format, the challenge-and-register flow, the lease table, the deny list, the verification code, and the `roymctl` commands that issue, deny and list passports.
  - **Home relay:** a registration endpoint on the coordinator, a dynamic allow-list for the Iroh relay, the substrate side (store, register, renew, warn), and the `roymctl` commands to give a passport to a substrate.
  - An amendment to ADR-0015 for the new abilities and the resource form (see Design). Written: [ADR-0015 A10](../../../decisions/0015-ucan-capability-model.md#a10-operator-passports-two-abilities-two-selectors-signed-use-limits).
  - Admission only for the home relay. A substrate with a lease has the same limits as any other. The mailbox use is the exception: its passport also raises quotas.
- Not in scope:
  - The WebRTC signalling server. It has no access control today (deferred backlog).
  - Per-passport quotas on connections or bandwidth, and payment (deferred backlog).
  - Cutting an open relay connection when a lease ends.
  - The mailbox use. It is specified in [service-mailbox](../service-mailbox/change.md), built on the core written here.

## Acceptance criteria

Core. Each passport use must pass these in its own tests.

| ID | Criterion | Test |
| --- | --- | --- |
| P-1 | The operator can issue a passport with an audience (owner DID), an ability, a resource, an expiry and use limits. | `crates/passport/src/issue/tests.rs`, `apps/roymctl/src/commands/coordinator/tests.rs` |
| P-2 | Registration is refused when: the chain does not end at an accepted operator root; the token is expired; the ability or resource does not match; the proven key is not bound to the subject (for the relay the two are the same key; for the mailbox the instance key is bound to the member master by its service-instance certificate); the proof of possession fails (challenge form: the signature is wrong, or the challenge is old or already used); or a use limit is reached. Each case has its own error. | `crates/passport/src/registrar/tests/refusals.rs`, `crates/passport/src/registrar/tests/possession.rs` |
| P-3 | A leaked passport is not enough. The caller must also prove that it holds the key bound to the subject of the owner's child token: by a challenge signature (relay) or by its transport identity (mailbox). | `crates/passport/src/registrar/tests/possession.rs`, `crates/passport/src/registrar/tests/refusals.rs` |
| P-4 | A caller whose lease has ended is refused when it connects the next time. A connection that is already open is not cut. The lease never outlasts the token or the `max_lease_secs` of its use. | |
| P-5 | After the operator's service restarts, users with a running lease keep their access. | |
| P-6 | The operator can deny a passport by its id. A denied caller is refused when it connects the next time. | |
| P-7 | Challenge form only. The registration endpoints accept a body of at most 16 KiB, keep at most 1,000 challenges (the oldest is dropped first), give each challenge a life of 60 seconds, do the size and shape checks before any signature check, and limit each source address to 10 requests per minute. | `crates/passport/src/challenge_store/tests.rs`, `crates/passport/src/limiter/tests.rs`, `crates/passport/src/wire/tests.rs` (core parts; the HTTP routes are B2) |
| P-8 | `roymctl` can issue a passport, deny a passport by id, and list the running leases. | issue: `apps/roymctl/src/commands/coordinator/tests.rs`, `apps/roymctl/tests/cli_args.rs`; deny and leases: B2 |
| P-9 | Challenge form only. The challenge signature covers a domain tag, the node DID of the host substrate of the service being registered with, the challenge, and a hash of the token. A signature made for one service is refused by another. | `crates/passport/src/registrar/tests/possession.rs` |
| P-10 | Challenge form only. The registration endpoints are served only over TLS. A service with passport access and no TLS config stops at startup. | |

Home relay

| ID | Criterion | Test |
| --- | --- | --- |
| H-1 | A substrate with a valid passport can register. After that the relay lets its node id connect, with no restart. | |
| H-2 | `max_nodes` limits how many distinct nodes use one passport. | |
| H-3 | An open relay (`access = "everyone"`) refuses registration with a clear error. | |
| H-4 | The substrate registers again when a third of the lease is left, retries with backoff, and logs a warning and sets a metric when 7 days or less of the passport life is left. | |
| H-5 | `roymctl` can hand a passport to a substrate and show its lease. | |
| H-6 | An unknown value of `[roles.coordinator] access` stops the coordinator at startup. Today any unknown string, including a typo, opens the relay to everyone. | `crates/core/src/config/access/tests.rs`, `crates/coordinator_iroh/tests/startup_config.rs` |
| H-7 | The substrate registers before the endpoint first connects to the relay. | |

## Design

### Core

**Token.** A `CapabilityToken` ([ADR-0015](../../../decisions/0015-ucan-capability-model.md), `crates/ucan`). The chain has two tokens:

1. The **passport**: issuer is the operator root, audience is the owner DID of the user.
2. The **child token**: the owner re-delegates the passport to the **subject**: the node key for the relay, the member master DID for the mailbox. Issuer is the owner DID, audience is the subject, `proofs` holds the passport. It has the same ability and resource, an expiry no later than the passport, and use limits no wider than the passport.

`verify_chain` already checks that each audience is the next issuer. The service calls it with the subject as the expected audience. No `DelegationCertificate` and no `ControllerAgreement` is part of this flow. The owner is only the middle link of the chain, and the owner key signs the child token when `roymctl` hands the passport over.

| Field | Value |
| --- | --- |
| capability | One ability on one resource that names the operator's service. |
| facts | The use limits, for example `{"max_nodes": 3}`. They are signed with the token. |
| expires | `expires_at_secs`. |

**Use limits are signed facts.** ADR-0015 A3 allows only the caveats `where`, `fields` and `can_delegate`, and a caveat must never need a data lookup. A count such as `max_nodes` needs one. So the limits go in the signed `facts`, and the **operator's service** checks them, not `verify_chain`. The ADR amendment records this.

**Passport id.** `token_id` is the SHA-256 of the canonical signed body of the passport (the body that its signature covers). `CapabilityToken` has no id field today. The deny list and the use counts use this id.

**Resource name.** A bare `substrate:<did>` matches every resource (`crates/ucan/src/capability.rs`). Use a selector form instead: `substrate:<host node did>/relay` for the relay and `substrate:<host node did>/mailbox` for the mailbox. The DID is always the node DID of the host substrate, whichever root (controller or node DID) issued the passport. The passport must name the ability and the selector exactly; entailment (for example from `substrate/admin`) is not accepted for the passport itself. The challenge signature (P-9) names the same node DID.

**ADR-0015 amendment.** Written as [ADR-0015 A10](../../../decisions/0015-ucan-capability-model.md#a10-operator-passports-two-abilities-two-selectors-signed-use-limits). It adds the abilities `relay/home` and `mailbox/use` (the ability set is closed, A2), names the two selectors (the selector form itself is A1), and reserves the facts keys for use limits of both uses.

**Operator root.** The root of the chain is the controller of the substrate that hosts the service. If that substrate has no controller, it is the node DID. The service accepts a passport rooted at either one. A coordinator role has no stable DID of its own: it makes a new random Iroh key at every start (`build_iroh_endpoint` in `crates/coordinator_iroh/src/coordinator.rs`). Passports stop working if the `ControllerAgreement` of the host substrate expires or changes.

**Subject.** The subject is the audience of the child token. For the relay it is the node key. For the mailbox it is the member master DID, which stays the same when the service moves. Leases and use counts are keyed on the subject, not on the key that proves possession, because an instance key changes at relocation.

**Proof of possession.** The core takes the proof as an input, in one of two forms:

- **Challenge form** (relay). The registration endpoints are plain HTTPS, so the caller must sign a fresh challenge. Two steps, so a request cannot be replayed:
  1. `challenge` returns a random value. It is kept in memory for 60 seconds and works once.
  2. `register` carries the child token and the caller's signature over `(domain tag, DID of this service, challenge, hash of the child token)`.
- **Transport form** (mailbox). Registration is a native call over Iroh. The transport has already checked the caller's instance key and its instance certificate (ADR-0016). The core receives that verified key. No challenge is needed, and P-7, P-9 and P-10 do not apply.

**Signer binding.** Each use gives the core a binding check: "is this proven key allowed to act for this subject?" For the relay the check is equality. For the mailbox it is the instance certificate whose master DID is the subject.

The operator's service then:

1. Checks the proof of possession. In the challenge form: reads the challenge, removes it, and verifies the signature against the subject.
2. Checks the signer binding.
3. Verifies the chain with `verify_chain`, with the subject as expected audience and the operator root as an accepted root. It checks the ability, the resource and the expiry.
4. Checks the use limits and that the passport id is not denied. The limits are read from the **passport** (the root token) only. A limit in the child token is ignored: `verify_chain` does not compare facts between tokens, so an owner could otherwise write `max_nodes: 1000` under a passport that says 3. Ignoring it means no comparison is needed.
5. Writes one lease row `(passport_id, owner_did, subject, lease_ends_at)`. The lease ends at the earliest of the child token expiry and the use's `max_lease_secs`. `max_lease_secs` is set for each use: relay default 24 hours; mailbox 30 days, the same as the queue lifetime, so a sleeping phone keeps its raised quota while its queue lives.

The response gives `lease_ends_at`. The service allows a caller when it has a row with a lease still running. It reads from memory, filled from SQLite at startup. In the challenge form, registration must be over TLS (P-10), because the token is sensitive. The info server runs TLS only when `[tls]` is set, so passport access on a relay requires it. In the transport form, the Iroh connection is already encrypted.

**What a use provides.** The ability and resource names, its facts, its `max_lease_secs`, its proof form, its signer binding, and how the lease is enforced (relay: the access function; mailbox: the quota lookup).

**Code.** The verification, the challenge store, the lease table and the deny list are one reusable module, not a copy for each use. It is a new crate, `crates/passport` (`syneroym-passport`), on top of `crates/ucan`. It needs SQLite and a request limiter, which `crates/ucan` should not carry.

**Rejected options.**
- **One-time registration with no expiry of access.** The token would give access forever. The token expiry must bound access.
- **A token tied to one device.** Owners run several devices, and a device key can change.
- **A `DelegationCertificate` or `ControllerAgreement` as the owner-to-key proof.** For a substrate the link to its owner is a `ControllerAgreement` (`crates/identity/src/substrate.rs:48`), which is not a certificate. The certificate scopes are only `routing`, `session-auth`, `service-instance` and `record-signing`. For a service, no signed owner-to-master link exists; the owner is only a local record (`crates/core/src/local_registry.rs:314`). A child token needs none of these.
- **Registration by `roymctl` directly.** The service must see the caller's own key signature, so the substrate must make the call.
- **Listing passport services in the registry.** The registry lists open services only.

### Use: home relay

**Ability and facts.** Ability `relay/home` on `substrate:<host node did>/relay`. Facts: `max_nodes`. The subject is the substrate's node key. The node key also signs the challenge, so the signer binding is equality.

**Endpoints.** On the coordinator's info HTTP server (the one with `/v1/info`): `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.

**Access setting.** A new value for `[roles.coordinator] access`: `"passport"`. Only nodes with a running lease are admitted. `"everyone"` and the static list keep their meaning. A table form admits both: `access = { list = ["<endpoint id>", ...], passport = true }` admits a node when either matches. Any other value stops the coordinator at startup (H-6). The strict parse ships first, with the relay plumbing of [registry-listed-coordinators](../registry-listed-coordinators/change.md), and already accepts the final shape.

**When access ends.** The Iroh relay checks access only when it accepts a connection (`iroh-relay-0.97.0/src/server/http_server.rs:750`), and no API to drop a connected client was found. So an expired lease or a denied passport ends access at the next reconnect. A live connection is not cut.

**Substrate.** The passport is kept by the substrate and used for the coordinator named by `parent_coordinator.iroh.url`. The substrate registers at start, before the endpoint first connects to the relay (otherwise the reconnect backoff of Iroh delays access after registration), and again when a third of the lease is left. Failures are retried with backoff. A relay confirmed as the home relay may still refuse the node, so registration success, not relay confirmation, is the proof of access.

**roymctl.** These names are a proposal. Check them against the existing `roymctl` layout.

- `roymctl coordinator passport issue (--from <name> | --node-key) --to <owner did> --expires-days <n> [--max-nodes <n>] [--out <file>]` writes a passport file and prints its id. Run by the operator. The signer is always named: `--from` is a local identity (normally the controller), `--node-key` is the node key, only for a host substrate with no controller (`D-B1-14`, `D-B1-21`). The host node DID comes from the global `--substrate` flag. The mailbox use gets its own `issue` variant on the same code.
- `roymctl coordinator passport deny <passport-id>` and `roymctl coordinator passport leases` deny a passport and list the running leases.
- `roymctl substrate home-relay set --coordinator <url> --passport <file>` runs on the owner's machine. It signs the child token for the node key of the substrate with the owner key, and hands it to the substrate through the control plane. The substrate registers itself.
- `roymctl substrate home-relay status` shows the lease and the expiry.

**Which home-relay source `home-relay set` applies to.** A substrate runs one home-relay source ([registry-listed-coordinators](../registry-listed-coordinators/change.md), D-A4-1): `Fixed` (from `parent_coordinator.iroh.url`), `Registry` (a background task picks a relay from the coordinator list), or none. The `Registry` task owns the `HomeRelay` handle and would undo a manual `set`. So the passport step (register, then insert the relay) applies to the `Fixed` source. On a node whose source is `Registry` or none, `home-relay set` must either refuse with a clear error, or first stop the resolver task. If it stops the task, it uses a cancellation token that the task checks between steps, never inside `HomeRelay::set` (a dropped `set` leaves the relay map and the watch channel out of step). The registry slice builds no way to stop the task (D-A4-15).

## Open questions

None open. Decided 2026-10-11:

- The shared core is the new crate `crates/passport` (see Design, Code).
- `access` may combine a static list with passports (see Access setting).
- The substrate keeps the child token in a state store set by `roymctl` through the control plane. No restart and no config path.

## Tasks

- [x] Amend ADR-0015: abilities `relay/home` and `mailbox/use`, the selector resource form, reserved facts keys (A10).
- [x] Core module: passport issue, child token, verify, both proof forms, signer binding, challenge store, lease table keyed on subject, deny list. Tests for each error in P-2 (both proof forms) and for P-9.
- [ ] Coordinator: challenge and register endpoints, dynamic access function, strict `access` parsing.
- [ ] Substrate: store the child token, register before first connect, renew, warn.
- [ ] `roymctl` commands (issue, deny, leases, home-relay set and status).
- [ ] Integration test: restricted relay, registration, expiry, restart, deny, reconnect after expiry. Use `common::alloc_ports`.
- [ ] Update the living docs listed below.

## Deviations

Core decisions (2026-10-14). The code is `crates/passport` (`syneroym-passport`), the `ucan` additions (`Ability::RELAY_HOME`, `Ability::MAILBOX_USE`, `CapabilityToken::token_id`, `CapabilityToken::sign`) and `roymctl coordinator passport issue`.

- `D-B1-1`: **the chain has exactly two tokens.** The child token has exactly one proof, the passport. The passport has no proofs. Each token has exactly one capability. Anything else is `malformed`. An operator who wants to admit a node directly still makes a passport for an owner, and the owner makes the child token.
- `D-B1-2`: **the core is form-agnostic.** A `PassportUse` fixes neither the proof form nor the binding form. The caller passes a `Proof` and a `SignerBinding` for each registration. The tests also run the two cross combinations.
- `D-B1-3`: **the challenge proof names its signer.** `Proof::Challenge` carries `signer_did`, and the signature is checked against it, not against the subject. The proven key is `signer_did` (challenge form) or `verified_did` (transport form). So "the proven key is not bound to the subject" (`not_bound`) is a real error in both forms.
- `D-B1-4`: **what the challenge signature covers.** The signer signs, with `Identity::sign_json` (RFC 8785), the object `{"domain": "syneroym-passport-challenge-v1", "service_did": <host node DID>, "ability": <ability>, "challenge": <64 hex>, "token_hash": <64 hex>}`. `token_hash` is the SHA-256 of the canonical JSON of the **whole presented child token, proofs and signatures included** (`presentation_hash`). The signed body of a token leaves out `proofs`, so a hash of the body alone would let someone place another passport under the same child token. `ability` separates the two uses on one host.
- `D-B1-5`: **the use limit counts running leases only.** `max_nodes` (and `max_services`) counts distinct subjects with a lease still running under the passport, not counting the registering subject. An ended lease frees its place. A renewal never counts against itself.
- `D-B1-6`: **a missing count key means no limit.** The relay has no open node limit. `PassportUse::open_count_limit` is `None` for both uses now; the mailbox use may set it.
- `D-B1-7`: **facts are checked at issue, read loosely at registration.** `issue_passport` refuses a key that is not a use limit of the use, and a value that is not a whole number of at least 1. At registration only the count key is read; other keys are ignored. A count key that is not a whole number is `malformed`. A value of 0 admits nobody (`limit_reached`).
- `D-B1-8`: **the lease end** is `min(child expiry, passport expiry, now + max_lease_secs)`. `verify_chain` does not check that a child ends before its parent, so the passport expiry is added. `child_token` also clamps the child expiry to the passport expiry.
- `D-B1-9`: **deny removes the leases.** `LeaseStore::deny` stores the id and deletes every lease of that passport in one transaction. A later registration is refused with `denied`. A deny of an id with no leases is still stored (deny in advance). There is no undeny (backlog row).
- `D-B1-10`: **one plain SQLite file per service.** The lease table holds passport ids, DIDs and times, no secret. `LeaseStore::open(dir, db_name, now)` follows `AlertStore::open`: WAL mode, `CREATE TABLE IF NOT EXISTS`, no version ladder. B2 picks the file name and directory. `LeaseStore::open(dir, db_name, use, now)` also binds the file to the ability of its use (table `passport_store`), and a file made for another use is an error. See `D-B1-23`.
- `D-B1-11`: **the clock is a parameter.** Every check and every function that makes a token takes `now_secs`. Callers pass `syneroym_passport::now_secs()`. The one exception is `DelegationCertificate::issue`, which reads the wall clock itself; its tests keep a margin of at least 5 seconds.
- `D-B1-12`: **the request limiter is in the core.** `RequestLimiter` is a fixed-window counter (10 requests per window of 60 seconds). It counts an IPv4 address, an IPv4-mapped IPv6 address as IPv4, and any other IPv6 address as its `/64`, because one sender owns a whole `/64`. It tracks at most 10,000 sources. When the table is full, the source whose window started earliest is dropped, so a new source is never refused for lack of room. A flood of fresh sources therefore cannot lock real users out. Dropping a source resets its own count, but only after 10,000 newer sources arrived. The window is fixed, so one source can make up to 20 requests in a short time across the edge of two windows. This is accepted. A sender with many `/64` prefixes can still make the challenge store drop live challenges (backlog row).
- `D-B1-13`: **the API is synchronous.** `LeaseStore` and `Registrar` are `Send + Sync`. `LeaseStore::is_admitted` reads memory only, so the Iroh relay access function can call it directly. B2 may call `grant` inside `spawn_blocking`.
- `D-B1-14`: **`roymctl` flag names** follow `identity issue-grant`: `--from`, `--to`, `--expires-days`, plus `--node-key`, `--max-nodes` and `--out`. The workspace has no duration parser, and a passport lives weeks or months, so the unit is days. The host node DID is the global `--substrate` (or `<dir>/substrate.key`).
- `D-B1-15`: **the DHT revocation check runs in the service, between `Registrar::check` and `Registrar::grant`.** ADR-0015 §2 and A7 require it on every chain edge. Two edges matter: `(operator, owner)` lets the operator cut one owner, and `(owner, subject)` lets the owner cut one node or member master. The lookup needs the network, and the sender chooses the DIDs, so it runs only after every offline check passed. `check` does every offline check and writes nothing (except that it uses up the challenge). `grant` does the deny check, the use-limit count and the write under one lock. The core defines `RegisterError::Revoked` (code `revoked`), so the error set is final. A revocation stops the next registration; a running lease stays until it ends. **Requirement on B2 and C8:** run `ucan_chain_not_revoked` (`crates/router/src/route_handler/io.rs`) on `checked.token()` between the two calls, and refuse with `Revoked`. B2 makes that function reachable from the coordinator. Code review of B2 and C8 must check that the lookup is there.
- `D-B1-16`: **the lease keeps the passport limits.** The row stores the passport facts as JSON (`limits`) and `registered_at`. `LeaseStore::lease_for(subject, now)` returns the lease that applies. With running leases under two passports, the newest registration wins (largest `registered_at`, then largest `lease_ends_at`, then smallest passport id).
- `D-B1-17`: **requirement on C4 and C8: the mailbox `register` request carries the whole instance certificate.** The router keeps only `VerifiedIdentity { master_did, temporary_did }` and also accepts the `routing` scope. So the mailbox passes `Proof::Transport { verified_did: <temporary_did> }` and `SignerBinding::InstanceCertificate(<certificate from the request>)`. The core checks the `service-instance` scope and `cert.temporary_did == verified_did`. C8 must not add a "the transport already checked it" binding form.
- `D-B1-18`: **requirement on B2: `deny` and `leases` go through the running service.** `LeaseStore` loads its index only at `open`, so a second process writing the file would not reach the running relay before a restart (that fails `P-6`). `roymctl coordinator passport deny` and `leases` call the process that owns the store. The core has no reload.
- `D-B1-19`: **a passport that forbids delegation is refused explicitly** (`invalid_chain`). When the owner is also an accepted root, `verify_chain` roots the child token on its own and never reads the passport caveat. The passport signature is still verified in that case; a test pins it.
- `D-B1-20`: **DIDs in a lease are canonical.** The z-base-32 decoder drops the 3 spare bits of the last character, so one key has several spellings (the tests find one). The core refuses a subject that is not exactly `derive_did_key` of its key (`malformed`). `issue_passport` and `child_token` refuse such an audience or subject too.
- `D-B1-21`: **`roymctl issue` never picks the signer.** `--from <name>` or `--node-key` is required (exactly one). Which roots a service accepts is decided in B2; a default of the node key would decide it now.
- `D-B1-22`: **the crate has its own `now_secs()`.** The coordinator and the substrate must not depend on the Roym product crate for a clock.
- **Requirement on B2:** remove the help line "A relay cannot use passports yet ..." from `roymctl coordinator passport issue` when the coordinator accepts `access = "passport"`.

Differences from the slice plan:

- The test fixture is `crates/passport/src/test_support.rs` (test builds only), not `registrar/tests.rs`, because the registrar tests and the wire tests both use it. `registrar/tests.rs` only names the topic files.
- `PassportCommands::Issue` holds a clap `Args` struct (`IssueArgs`) with the signer group on it, so `handle_issue` takes one argument instead of eight.
- Only `require_key_present` in `roymctl`'s `identity.rs` became `pub(crate)`; `identity_key_path` was not needed.
- The expiry boundary test uses a life of 300 seconds, not 3,600: the mailbox fixture's instance certificate lives 600 seconds and would end first.
- A few extra tests: a zero passport life, a child token made from a token that is not a passport, ended challenges leaving the store, `with_max_lease_secs`, and the error body code.

## Close-out

Living docs to update when this ships:

- [system-architecture.md](../../../system-architecture.md): config table (`access`), coordinator endpoints, security section.
- [system-requirements-spec.md](../../../system-requirements-spec.md): Relay section.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "passport", "child token" and "lease".
- A user-facing note for relay operators, in the developer guide.

The backlog rows "Home-relay passport for the WebRTC signalling server" and "Operator passport: per-passport quotas and payment" are added with this change.
- `D-B1-23`: **a lease store belongs to one use, and a registration belongs to one registrar.** A lease row has no ability, so a store shared by two uses would let a mailbox lease (subject: a member master DID) admit that key on the relay, and would mix the two use counts. `LeaseStore::open` and `open_in_memory` take the `PassportUse` and keep its ability in the file. `Registrar::new` refuses a store of another use. A `CheckedRegistration` keeps the id of the `Registrar` that made it, and `grant` on another registrar returns `internal` (a programming error, not a user error). `Registrar::new` also refuses a host node DID or an accepted root that is not a canonical DID, so a badly spelled config value fails at start and not as a misleading `untrusted_root` on every registration.
- `D-B1-24`: **response types accept unknown fields.** Only `RegisterRequest` refuses them. A substrate reads responses from coordinators of other operators, and a newer coordinator may add a field. A store error is sent with the fixed message `internal error`; the service must log the error before it converts it (the core does not log).
- `D-B1-25`: **a challenge that is too old gets one of two codes.** `challenge_expired` comes back only while no later `issue` has dropped the challenge; after that it is `unknown_challenge`. Clients must treat both the same way: ask for a new challenge and try again. Written in the `wire` module.
- `D-B1-26`: **two `issue` runs with the same options in the same second make the same passport.** Ed25519 signatures are deterministic, so the two files are byte-identical and have one id. `deny` and `max_nodes` then cover both. A nonce would need a reserved facts key, which is a change to ADR-0015 A10. The `issue` help says so.
- `D-B1-27`: **`child_token` refuses a life of 0 seconds**, like `issue_passport`. A token that ends when it starts can never verify.
- `D-B1-28`: **`write_secret_file` sets mode `0600` on an existing file too.** The `mode` flag of `open` applies only when the file is created, so `--out` onto a `0644` file kept `0644`.
