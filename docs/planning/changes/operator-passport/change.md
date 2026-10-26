---
status: done
living-docs-touched:
  - docs/developer-guide.md
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/planning/traceability-matrix.md
  - docs/TERMINOLOGY.md
  - docs/planning/deferred-backlog.md
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
| P-4 | A caller whose lease has ended is refused when it connects the next time. A connection that is already open is not cut. The lease never outlasts the token or the `max_lease_secs` of its use. | `crates/coordinator_iroh/tests/relay_passport.rs` |
| P-5 | After the operator's service restarts, users with a running lease keep their access. | `crates/coordinator_iroh/tests/relay_passport.rs` |
| P-6 | The operator can deny a passport by its id. A denied caller is refused when it connects the next time. | `crates/coordinator_iroh/tests/relay_passport.rs`, `crates/substrate/tests/relay_passport_admin.rs` |
| P-7 | Challenge form only. The registration endpoints accept a body of at most 16 KiB, keep at most 1,000 challenges (the oldest is dropped first), give each challenge a life of 60 seconds, do the size and shape checks before any signature check, and limit each source address to 10 requests per minute. | `crates/passport/src/challenge_store/tests.rs`, `crates/passport/src/limiter/tests.rs`, `crates/passport/src/wire/tests.rs`, `crates/coordinator_iroh/tests/relay_passport.rs` |
| P-8 | `roymctl` can issue a passport, deny a passport by id, and list the running leases. | issue: `apps/roymctl/src/commands/coordinator/tests.rs`, `apps/roymctl/tests/cli_args.rs`; deny and leases: `apps/roymctl/src/commands/coordinator/tests.rs`, `crates/substrate/tests/relay_passport_admin.rs` |
| P-9 | Challenge form only. The challenge signature covers a domain tag, the node DID of the host substrate of the service being registered with, the challenge, and a hash of the token. A signature made for one service is refused by another. | `crates/passport/src/registrar/tests/possession.rs` |
| P-10 | Challenge form only. The registration endpoints are served only over TLS. A service with passport access and no TLS config stops at startup. | `crates/coordinator_iroh/tests/startup_config.rs`, `crates/coordinator_iroh/tests/relay_passport.rs` |

Home relay

| ID | Criterion | Test |
| --- | --- | --- |
| H-1 | A substrate with a valid passport can register. After that the relay lets its node id connect, with no restart. | `crates/coordinator_iroh/tests/relay_passport.rs` |
| H-2 | `max_nodes` limits how many distinct nodes use one passport. | `crates/coordinator_iroh/tests/relay_passport.rs` |
| H-3 | An open relay (`access = "everyone"`) refuses registration with a clear error. | `crates/coordinator_iroh/tests/relay_passport.rs` |
| H-4 | The substrate registers again when a third of the lease is left, retries with backoff, and logs a warning and sets a metric when 7 days or less of the passport life is left. | `crates/substrate/src/runtime/home_relay/passport/tests/renew.rs`, `crates/substrate/src/runtime/home_relay/passport/tests/refusals.rs` |
| H-5 | `roymctl` can hand a passport to a substrate and show its lease. | `apps/roymctl/src/commands/substrate/home_relay/tests.rs`, `apps/roymctl/tests/cli_args.rs`, `crates/substrate/tests/home_relay_passport_e2e.rs` |
| H-6 | An unknown value of `[roles.coordinator] access` stops the coordinator at startup. Today any unknown string, including a typo, opens the relay to everyone. | `crates/core/src/config/access/tests.rs`, `crates/coordinator_iroh/tests/startup_config.rs` |
| H-7 | The substrate registers before the endpoint first connects to the relay. | `crates/substrate/src/runtime/home_relay/passport/tests/boot.rs`, `crates/substrate/tests/home_relay_passport_e2e.rs` |

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

**Operator root.** The root of the chain is determined from the host substrate identity at coordinator startup. When the host is owned, the verified controller DID is the root. When the host has no verified controller, the host node DID is the root (`D-B2-1`). Exactly one root is accepted, never both, and never `[iam].admin_ucan_root`. A coordinator role has no stable DID of its own: it makes a new random Iroh key at every start (`build_iroh_endpoint` in `crates/coordinator_iroh/src/coordinator.rs`). The accepted root is read once at startup; a controller agreement that expires or changes while the coordinator is running takes effect on the next restart, and no lease is dropped while the coordinator runs.

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

**Who can reach a leased node.** A passport relay is a closed-group relay (`D-B2-15`). The Iroh relay checks access for every client that connects, and a peer that sends to a node through that node's home relay connects to it as a client (a visitor). The access function sees only the endpoint id, so it cannot tell a home client from a visitor. Only leased, listed or always-admitted endpoints reach each other through the relay. Other substrates, `roymctl` and `SyneroymClient` reach a leased node only through a direct path, or if they hold a lease too.

**Substrate.** The passport is kept by the substrate and used for the coordinator named by `parent_coordinator.iroh.url`. The substrate registers at start, before the endpoint first connects to the relay (otherwise the reconnect backoff of Iroh delays access after registration), and again when a third of the lease is left. Failures are retried with backoff. A relay confirmed as the home relay may still refuse the node, so registration success, not relay confirmation, is the proof of access.

**roymctl.** These names are a proposal. Check them against the existing `roymctl` layout.

- `roymctl coordinator passport issue (--from <name> | --node-key) --to <owner did> --expires-days <n> [--max-nodes <n>] [--out <file>]` writes a passport file and prints its id. Run by the operator. The signer is always named: `--from` is a local identity (normally the controller), `--node-key` is the node key, only for a host substrate with no controller (`D-B1-14`, `D-B1-21`). The host node DID comes from the global `--substrate` flag. The mailbox use gets its own `issue` variant on the same code.
- `roymctl coordinator passport deny <passport-id>` and `roymctl coordinator passport leases` deny a passport and list the running leases.
- `roymctl substrate home-relay set --info-url <url> --passport <file> [--relay-url <url>] [--ca-cert <file>] [--as <identity>]` runs on the owner's machine. It signs the child token for the node key of the substrate with the owner key, and hands it to the substrate through the control plane. `--info-url` is the coordinator's info server base URL (`https` only). `--relay-url` overrides the target relay URL if different from the node's configured relay (the bootstrap order: set the passport on the live node on an open relay, then restart the node configured with the passport relay). `--ca-cert` supplies an extra root certificate (PEM) for private or self-signed coordinator info servers.
- `roymctl substrate home-relay status` shows the active status, relay URL, info URL, passport ID, owner DID, access expiry, running lease expiry, last registration attempt time, and any refusal error.

**Which home-relay source `home-relay set` applies to.** A substrate runs one home-relay source ([registry-listed-coordinators](../registry-listed-coordinators/change.md), D-A4-1): `Fixed` (from `parent_coordinator.iroh.url`), `Registry` (a background task picks a relay from the coordinator list), or none. The `Registry` task owns the `HomeRelay` handle and would undo a manual `set`. So the passport step (register, then insert the relay) applies to the `Fixed` source. On a node whose source is `Registry` or none, `home-relay set` must either refuse with a clear error, or first stop the resolver task. If it stops the task, it uses a cancellation token that the task checks between steps, never inside `HomeRelay::set` (a dropped `set` leaves the relay map and the watch channel out of step). The registry slice builds no way to stop the task (D-A4-15).

## Open questions

None open. Decided 2026-10-11:

- The shared core is the new crate `crates/passport` (see Design, Code).
- `access` may combine a static list with passports (see Access setting).
- The substrate keeps the child token in a state store set by `roymctl` through the control plane. Switching an existing node to a passport relay requires configuring `parent_coordinator.iroh.url` and restarting (Q2).

## Tasks

- [x] Amend ADR-0015: abilities `relay/home` and `mailbox/use`, the selector resource form, reserved facts keys (A10).
- [x] Core module: passport issue, child token, verify, both proof forms, signer binding, challenge store, lease table keyed on subject, deny list. Tests for each error in P-2 (both proof forms) and for P-9.
- [x] Coordinator: challenge and register endpoints, dynamic access function, strict `access` parsing.
- [x] Substrate: store the child token, register before first connect, renew, warn.
- [x] `roymctl` commands (issue, deny, leases done; home-relay set and status are B3).
- [x] Integration test: restricted relay, registration, expiry, restart, deny, reconnect after expiry. Use `common::alloc_ports`.
- [x] Update the living docs listed below.

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
- `D-B1-10`: **one plain SQLite file per service.** The lease table holds passport ids, DIDs and times, no secret. `LeaseStore::open` follows `AlertStore::open`: WAL mode, `CREATE TABLE IF NOT EXISTS`, no version ladder. B2 picks the file name and directory. `LeaseStore::open(dir, db_name, use, now)` also binds the file to the ability of its use (table `passport_store`), and a file made for another use is an error. See `D-B1-23`.
- `D-B1-11`: **the clock is a parameter.** Every check and every function that makes a token takes `now_secs`. Callers pass `syneroym_passport::now_secs()`. The one exception is `DelegationCertificate::issue`, which reads the wall clock itself; its tests keep a margin of at least 5 seconds.
- `D-B1-12`: **the request limiter is in the core.** `RequestLimiter` is a fixed-window counter (10 requests per window of 60 seconds). It counts an IPv4 address, an IPv4-mapped IPv6 address as IPv4, and any other IPv6 address as its `/64`, because one sender owns a whole `/64`. It tracks at most 10,000 sources. When the table is full, the source whose window started earliest is dropped, so a new source is never refused for lack of room. A flood of fresh sources therefore cannot lock real users out. Dropping a source resets its own count, but only after 10,000 newer sources arrived. The window is fixed, so one source can make up to 20 requests in a short time across the edge of two windows. This is accepted. A sender with many `/64` prefixes can still make the challenge store drop live challenges (backlog row). Slice C4 made `RequestLimiter` generic over the key type `K: Hash + Eq + Clone`, so the mailbox role reuses it for per-sender DID and per-node rate limiting.
- `D-B1-13`: **the API is synchronous.** `LeaseStore` and `Registrar` are `Send + Sync`. `LeaseStore::is_admitted` reads memory only, so the Iroh relay access function can call it directly. B2 may call `grant` inside `spawn_blocking`.
- `D-B1-14`: **`roymctl` flag names** follow `identity issue-grant`: `--from`, `--to`, `--expires-days`, plus `--node-key`, `--max-nodes` and `--out`. The workspace has no duration parser, and a passport lives weeks or months, so the unit is days. The host node DID is the global `--substrate` (or `<dir>/substrate.key`).
- `D-B1-15`: **the DHT revocation check runs in the service, between `Registrar::check` and `Registrar::grant`.** ADR-0015 §2 and A7 require it on every chain edge. Two edges matter: `(operator, owner)` lets the operator cut one owner, and `(owner, subject)` lets the owner cut one node or member master. The lookup needs the network, and the sender chooses the DIDs, so it runs only after every offline check passed. `check` does every offline check and writes nothing (except that it uses up the challenge). `grant` does the deny check, the use-limit count and the write under one lock. The core defines `RegisterError::Revoked` (code `revoked`), so the error set is final. A revocation stops the next registration; a running lease stays until it ends. **Requirement on B2 and C8:** run `ucan_chain_not_revoked` (`crates/router/src/route_handler/io.rs`) on `checked.token()` between the two calls, and refuse with `Revoked`. B2 makes that function reachable from the coordinator. Code review of B2 and C8 must check that the lookup is there.
- `D-B1-16`: **the lease keeps the passport limits.** The row stores the passport facts as JSON (`limits`) and `registered_at`. `LeaseStore::lease_for(subject, now)` returns the lease that applies. With running leases under two passports, the newest registration wins (largest `registered_at`, then largest `lease_ends_at`, then smallest passport id).
- `D-B1-17`: **requirement on C4 and C8: the mailbox `register` request carries the whole instance certificate.** The router keeps only `VerifiedIdentity { master_did, temporary_did }` and also accepts the `routing` scope. So the mailbox passes `Proof::Transport { verified_did: <temporary_did> }` and `SignerBinding::InstanceCertificate(<certificate from the request>)`. The core checks the `service-instance` scope and `cert.temporary_did == verified_did`. C8 must not add a "the transport already checked it" binding form. Changed by C4 (`D-C4-1`): the router passes the verified certificate in `CallerContext.proof.delegation_json`, so `register` has no certificate in its body. C8 takes the certificate for `SignerBinding::InstanceCertificate` from there. The binding form and the rule against a 'the transport already checked it' form are unchanged.
- `D-B1-18`: **requirement on B2: `deny` and `leases` go through the running service.** `LeaseStore` loads its index only at `open`, so a second process writing the file would not reach the running relay before a restart (that fails `P-6`). `roymctl coordinator passport deny` and `leases` call the process that owns the store. The core has no reload.
- `D-B1-19`: **a passport that forbids delegation is refused explicitly** (`invalid_chain`). When the owner is also an accepted root, `verify_chain` roots the child token on its own and never reads the passport caveat. The passport signature is still verified in that case; a test pins it.
- `D-B1-20`: **DIDs in a lease are canonical.** The z-base-32 decoder drops the 3 spare bits of the last character, so one key has several spellings (the tests find one). The core refuses a subject that is not exactly `derive_did_key` of its key (`malformed`). `issue_passport` and `child_token` refuse such an audience or subject too.
- `D-B1-21`: **`roymctl issue` never picks the signer.** `--from <name>` or `--node-key` is required (exactly one). Which roots a service accepts is decided in B2; a default of the node key would decide it now.
- `D-B1-22`: **the crate has its own `now_secs()`.** The coordinator and the substrate must not depend on the Roym product crate for a clock.
- **Requirement on B2:** remove the help line "A relay cannot use passports yet ..." from `roymctl coordinator passport issue` when the coordinator accepts `access = "passport"`.
- `D-B1-23`: **a lease store belongs to one use, and a registration belongs to one registrar.** A lease row has no ability, so a store shared by two uses would let a mailbox lease (subject: a member master DID) admit that key on the relay, and would mix the two use counts. `LeaseStore::open` and `open_in_memory` take the `PassportUse` and keep its ability in the file. `Registrar::new` refuses a store of another use. A `CheckedRegistration` keeps the id of the `Registrar` that made it, and `grant` on another registrar returns `internal` (a programming error, not a user error). `Registrar::new` also refuses a host node DID or an accepted root that is not a canonical DID, so a badly spelled config value fails at start and not as a misleading `untrusted_root` on every registration.
- `D-B1-24`: **response types accept unknown fields.** Only `RegisterRequest` refuses them. A substrate reads responses from coordinators of other operators, and a newer coordinator may add a field. A store error is sent with the fixed message `internal error`; the service must log the error before it converts it (the core does not log).
- `D-B1-25`: **a challenge that is too old gets one of two codes.** `challenge_expired` comes back only while no later `issue` has dropped the challenge; after that it is `unknown_challenge`. Clients must treat both the same way: ask for a new challenge and try again. Written in the `wire` module.
- `D-B1-26`: **two `issue` runs with the same options in the same second make the same passport.** Ed25519 signatures are deterministic, so the two files are byte-identical and have one id. `deny` and `max_nodes` then cover both. A nonce would need a reserved facts key, which is a change to ADR-0015 A10. The `issue` help says so.
- `D-B1-27`: **`child_token` refuses a life of 0 seconds**, like `issue_passport`. A token that ends when it starts can never verify.
- `D-B1-28`: **`write_secret_file` sets mode `0600` on an existing file too.** The `mode` flag of `open` applies only when the file is created, so `--out` onto a `0644` file kept `0644`.

Coordinator decisions (2026-10-15). The code is `crates/coordinator_iroh`, `crates/coordinator`, `crates/substrate`, `crates/control_plane`, `apps/roymctl` (`deny`, `leases`), and the shared types in `crates/core`.

- `D-B2-1`: **the passport root follows ADR-0015 A10 point 4.** The relay accepts a passport rooted at the verified controller of the host substrate, or at the host node DID when the host has no verified controller. Exactly one root, never both, and never `[iam].admin_ucan_root` (A10: "It is not the node-wide `admin_ucan_root`"). An unverified controller is not a controller (the same rule as `substrate/admin`). So on a host with `admin_ucan_root` and no verified controller, the node key issues passports and the admin root denies them (`D-B2-6`); on a host with a verified controller, both are the controller. The root is read once at startup; a controller agreement that ends or changes while the substrate runs keeps the old root until restart (the same as `substrate/admin`, see the `roymctl substrate claim --expires-days` help). Backlog row. `roymctl coordinator passport issue --node-key` keeps its help text ("only for a host substrate that has no controller"), which is right under this rule.
- `D-B2-2`: **`[tls]` is required whenever `access` accepts passports**, for every coordinator role: also when `enable_relay = false`, and also when the role has no `[roles.coordinator.iroh]` section (only WebRTC, or nothing). The check runs in `EcosystemCoordinator::init` before any coordinator is built, and again in `CoordinatorIroh::init` (which tests and `smoke-tests` call directly). Both call one function, `CoordinatorAccess::check_tls` in `crates/core`. The error names `[tls]`. When the role runs no Iroh relay, passport access only logs a warning ("has no effect"), opens no lease store, and the endpoints (if the info server runs) answer `not_accepted`.
- `D-B2-3`: **the coordinator's own endpoint and the host node are always admitted, under every restricted form** (list, passport, both). Without this, the coordinator's random endpoint has no lease and no home relay. The host node has the same problem: a substrate that runs a passport relay and uses it as its own home relay would be refused by its own relay. One rule for all restricted forms is simpler, and it also fixes the same gap for the list form. It replaces `D-A3-10` of the coordinators change doc: now the coordinator's own endpoint is counted whenever it uses its own relay (no parent relay). It changes the `relay.clients` count of a list-access relay by one, and the `max_relay_clients = 1` warning of `check_relay_cap` fires for every access form. Consequence for B3: a test whose node uses its own relay is always admitted, so it would pass even if registration did nothing. B3's tests must register a node with a separate passport relay node.
- `D-B2-4`: **two more refusal codes in the core**, `RegisterError::RateLimited` (`rate_limited`) and `RegisterError::NotAccepted` (`not_accepted`). `D-B1-15` says the error set is final; it is not, because `H-3` ("refuses registration with a clear error") and the request limit (`P-7`) need codes. Keeping every wire code in one enum lets B3 match on one list.
- `D-B2-5`: **HTTP status of each refusal.** `request_too_large` 413; `malformed`, `unknown_challenge`, `challenge_expired` 400; `rate_limited` 429; `internal` 500; every other code 403 (`bad_signature`, `not_bound`, `untrusted_root`, `broken_chain`, `wrong_capability`, `expired`, `invalid_chain`, `denied`, `revoked`, `limit_reached`, `not_accepted`). A client decides by `code`, not by status.
- `D-B2-6`: **`deny` and `leases` go through the control plane**, as two methods of the existing `security` interface, gated by `substrate/admin` like the other methods there. They are declared in the WIT `security` interface, like every other control-plane method. Rejected: signed admin requests on the info server. That needs a new signed-request format and replay rule, and it would be a second admin path beside the control plane. The control plane is already how `roymctl` runs every node-owner verb, and B3 uses it too. Cost: an unowned host (no controller and no `[iam].admin_ucan_root`) cannot run `deny` or `leases`, although its node key can issue passports. This matches the rest of the system (an unowned substrate refuses every admin verb). Backlog row.
- `D-B2-7`: **the lease file** is `<storage.db_dir>/relay_passports.db`. The directory is created if missing.
- `D-B2-8`: **the revocation lookup uses the host's registry**: `RegistryClient::new(config.substrate.enable_bep0044_dht, config.substrate.registry_url.clone())`, the same source the node's own router uses for every other UCAN chain (`crates/router/src/route_handler.rs:372`). Not the coordinator's `[roles.coordinator.iroh] community_registry_url`, which is where the coordinator registers itself. A failed lookup counts as "not revoked"; this is the existing behaviour of `ucan_chain_not_revoked`. Backlog row.
- `D-B2-9`: **the request limiter counts the TCP peer address.** `X-Forwarded-For` is not read: anyone could set it. Behind a reverse proxy, all users share the proxy's address and one limit. Backlog row.
- `D-B2-10`: **one request limiter for both endpoints.** A registration costs two requests (challenge, register), so one address can register five times a minute. A `not_accepted` answer does not count (no limiter exists then).
- `D-B2-11`: **the challenge request body is ignored.** `POST /v1/home-relay/challenge` takes no body. Its answer is `ChallengeResponse` unchanged.
- `D-B2-12`: **body reading.** The register route has `DefaultBodyLimit::max(MAX_REQUEST_BYTES)`, and the handler takes `Result<Bytes, BytesRejection>`, so an oversize body gets the JSON `request_too_large` body (status 413) and not axum's plain-text one. The body is read (at most 16 KiB) before the limiter runs; the size and shape checks still run before any signature check.
- `D-B2-13`: **`/v1/info` `identity` is always reported now**: `{ "did": <host node DID>, "controller_status": "verified" | "unverified" | "none" }` (the M02 meaning; the values are `SubstrateIdentityStatus` in lower case).
- `D-B2-14`: **`max_lease_secs` is not configurable.** The relay uses the 24 hours of `RELAY_HOME`. No config key (no user asked for one).
- `D-B2-15`: **a passport relay is a closed-group relay.** The Iroh relay checks access for every client that connects, and a peer that sends to a node through that node's home relay connects to it as a client (a visitor). The access function sees only the endpoint id, so it cannot tell a home client from a visitor. So on a relay with passport (or list) access, only leased, listed or always-admitted endpoints can reach each other through it. Other substrates, `roymctl` and `SyneroymClient` reach a leased node only through a direct path, or if they hold a lease too. This goes beyond `H-1` (which only says the leased node may connect). Backlog row.
- `D-B2-16`: **the root rule lives in a neutral module.** `HostIdentity` and its `passport_root()` go in `crates/core/src/host_identity.rs`, not in `coordinator_info`, because A10's per-service root rule is shared with the dedicated mailbox. C8 reuses `HostIdentity::passport_root` for `MAILBOX_USE` and must not write its own. `/v1/info` gets its `IdentityInfo` from `HostIdentity::identity_info`.
- `D-B2-17`: **`LeaseInfo` has no `limits`.** The WIT record of the lease list has no JSON map type, and the operator issued the limits and knows them. The lease row still keeps them (`D-B1-16`) for the mailbox quota lookup.
- `D-B2-18`: **revocation lookup overall timeout of 15 seconds.** When checking whether a passport chain is revoked, the HTTP registry lookup and DHT resolution are bounded by a 15-second timeout. If the lookup exceeds 15 seconds, it times out and is treated as not revoked (`D-B2-8`), logging a warning. This bounds registration latency under degraded DHT or network conditions.

Requirements on B3:
- (a) the substrate signs the challenge for the host node DID read from the passport's resource (`substrate:<did>/relay`), not from `/v1/info`;
- (b) treat `unknown_challenge` and `challenge_expired` the same (`D-B1-25`);
- (c) `not_accepted`, `untrusted_root`, `wrong_capability`, `denied`, `revoked`, `limit_reached` and `expired` will not change by retrying soon, while `rate_limited`, `internal` and network errors are retried with backoff;
- (d) the response types accept unknown fields;
- (e) **the registering node must use a separate passport relay node in every B3 test**: a node is always admitted to its own relay (`D-B2-3`), so a test with one node that uses its own relay passes even when registration does nothing;
- (f) the clients and peers in B3's tests reach the leased node only if they are leased too, or over a direct path (`D-B2-15`).
- (g) client HTTP request timeout for `POST /v1/home-relay/register` must be set above the coordinator's 15-second revocation lookup timeout (at least 25 seconds), so the client does not abort a registration while the coordinator is still completing the lookup.

Differences from the slice plan:

- The test fixture is `crates/passport/src/test_support.rs` (test builds only), not `registrar/tests.rs`, because the registrar tests and the wire tests both use it. `registrar/tests.rs` only names the topic files.
- `PassportCommands::Issue` holds a clap `Args` struct (`IssueArgs`) with the signer group on it, so `handle_issue` takes one argument instead of eight.
- Only `require_key_present` in `roymctl`'s `identity.rs` became `pub(crate)`; `identity_key_path` was not needed.
- The expiry boundary test uses a life of 300 seconds, not 3,600: the mailbox fixture's instance certificate lives 600 seconds and would end first.
- A few extra tests: a zero passport life, a child token made from a token that is not a passport, ended challenges leaving the store, `with_max_lease_secs`, and the error body code.
- When `enable_bep0044_dht` is enabled, the coordinator instantiates a second `RegistryClient` (and underlying `DhtClient`) for the passport registrar to query the host node's registry for revocations, which opens a second local UDP socket. Sharing the DHT client between the coordinator and the registrar is recorded in the deferred backlog.
- A child coordinator generates a fresh, random Iroh key at each startup, so it cannot be listed in advance and cannot hold a passport lease; a list-access or passport-access parent relay always refuses it, so only an open relay (`access = "everyone"`) can serve as a parent relay today.

Substrate decisions (2026-10-16). The code is `crates/substrate`, `crates/passport` (presented token, wire types), `crates/control_plane`, `crates/wit_interfaces`, and `apps/roymctl` (`substrate home-relay set`, `status`).

- `D-B3-1`: **the passport state file** is `<storage.db_dir>/home_relay_passport.json`. It stores `StoredPassport` (`token`, `info_url`, `relay_url`, and optional `ca_cert_pem`). It holds no registration status or lease state.
- `D-B3-2`: **`home-relay set` applies to `Fixed` relay source.** If the node has no fixed relay configured (`parent_coordinator.iroh.url = None`, such as when using the community registry resolver), `security.home-relay-set` refuses with JSON-RPC error code `-32601` (`MethodNotFound`) and a message naming `parent_coordinator.iroh.url`.
- `D-B3-3`: **client HTTP requests disable connection pooling (`pool_max_idle_per_host(0)`).** In `LiveIo`, `reqwest::Client::builder().pool_max_idle_per_host(0)` ensures client connections are not pooled across relay restarts or rebinds.
- `D-B3-4`: **`CoordinatorIroh::shutdown` awaits the aborted `http_info_handle`.** When shutting down the coordinator, the aborted background info server task is awaited so that its socket is cleanly unbound before a subsequent restart binds the same port.
- `D-B3-5`: **when passport access ends, `warn!` is logged and the metric gauge is cleared.** When the passport or child token expires or is denied, the substrate logs a warning and sets `substrate.home_relay.access_ends_at_secs` to 0.
- `D-B3-6`: **`inspect_child_token` offline checks.** The substrate inspects child tokens before storage and presentation, confirming the subject, host DID from resource, token shape, and signature chain.
- `D-B3-7`: **where the substrate registers (Q1).** `set` takes `--info-url <https url>` (required) and `--relay-url <url>` (optional; default: the node's `parent_coordinator.iroh.url`). The substrate stores both with the token. The stored passport is active only while its relay URL equals the configured `parent_coordinator.iroh.url`. Otherwise it is kept but not used, and `status` reports it as inactive.
- `D-B3-8`: **bootstrap order to switch to a passport relay (Q2).** While the node still uses a reachable relay or local network, run `home-relay set --relay-url <passport relay> ...`. The passport is stored as inactive. Then edit `parent_coordinator.iroh.url` in config and restart the substrate. At boot the node registers before inserting the relay.
- `D-B3-9`: **TLS trust for private info servers (Q3).** `set --ca-cert <pem file>` accepts an optional PEM certificate, validated at set time, stored alongside the passport, and added to the client root store for requests to the info server URL.
- `D-B3-10`: **boot round retry budget and relay insertion (Q4).** At boot with an active passport, the agent tries registration up to 3 times (backoff 5 s, then 10 s) bounded by a 30-second budget (`BOOT_BUDGET`). It signals `boot_done` (inserting the relay into the router) after the first success, after a non-retriable refusal, after the 3rd failure, or when the 30 s budget expires, continuing retries in the background.
- `D-B3-11`: **renewal scheduling near access end (Q5).** When `lease_ends_at >= access_ends_at`, the agent does not schedule a renewal since the lease cannot be extended beyond access end, but plans a wake-up at `access_ends_at` so that it resets the metric gauge, logs the access-ended warning, and updates `last_error` to `expired` once access ends. Renewal attempts are never scheduled sooner than 60 seconds after the previous attempt.
- `D-B3-12`: **refusal classification and backoff (Q7).** Refusal codes are classified into three classes (`syneroym_passport::wire::RetryClass`): `NewChallenge` (`unknown_challenge`, `challenge_expired`) re-requests a challenge immediately (once per attempt); `Backoff` (`rate_limited`, `internal`, network errors, invalid JSON, and unknown future codes) retries with exponential backoff (5 s doubling to 10 min); `Settled` (other known codes like `denied`, `revoked`, `expired`, `limit_reached`) warns and sleeps 1 hour before rechecking, stopping completely once access has ended.
- `D-B3-13`: **child token signing identity (Q8).** `roymctl substrate home-relay set` signs the child token using the `--as <name>` identity, which must match the passport audience DID and hold `substrate/admin` on the node. The child token's lifetime matches the passport expiry.
- `D-B3-14`: **control-plane method names (Q9).** Two owner-only methods on the `security` interface: `home-relay-set` and `home-relay-status`, mirroring the CLI verbs.
- `D-B3-15`: **synchronous initial attempt in `home-relay-set` (Q10).** `security.home-relay-set` validates and stores the token, runs one synchronous registration attempt (if active) bounded by HTTP request timeouts, and returns the resulting status so the caller sees success or refusals immediately.
- `D-B3-16`: **single passport storage and deletion (Q11).** A node stores at most one home-relay passport. `home-relay set` replaces the existing passport and resets backoff. No runtime `clear` verb is provided; removing the passport requires deleting `home_relay_passport.json` and restarting.
- `D-B3-17`: **relay certificate trust vs info server trust (Q13).** Substrate Iroh endpoints trust WebPKI roots for relay connections (`CaRootsConfig::default`). The optional `--ca-cert` applies exclusively to the HTTPS info server client. Relays with private or self-signed TLS certificates cannot be used as home relays without WebPKI trust.
- `D-B3-18`: **coordinator role warning on passport parent relay (Q14).** If a node runs a coordinator Iroh role and its `parent_coordinator.iroh.url` points to a passport-restricted relay, the passport agent logs a warning at boot because the coordinator role's ephemeral random key will be refused by the parent relay.
- `D-B3-19`: **expiry and renewal test separation (Q15).** The acceptance requirement "reconnect after expiry" is verified in two parts: (1) token expiry leading to lease termination, restored after setting a fresh passport (verified in multi-node integration test `home_relay_passport_e2e.rs`), and (2) lease expiration while the passport remains valid, automatically triggering renewal and reconnect (verified in paused-clock unit test `an_ended_lease_is_renewed_while_the_token_runs`).
- `D-B3-20`: **host suspend / monotonic sleep protection.** Sleep intervals are capped at 5 minutes (`SLEEP_CAP`), with target wall-clock comparison re-evaluated on each loop iteration, ensuring renewal occurs promptly after a host resumes from suspend or clock jump.

Differences from the slice plan:

- `restart_in_place` shared test helper in `tests/common/node.rs` reboots a test node in place, preserving its allocated ports and directory state.
- Atomic write temporary file naming convention `<file>.tmp` in `write_file_atomic` (`crates/substrate/src/runtime/home_relay.rs`).
- CA certificate PEM validation test build in `from_request` (`reqwest::Certificate::from_pem`).
- `D-C8-*` in the [service-mailbox change](../service-mailbox/change.md): the mailbox use of the passport (`mailbox/use`), including the 4 s revocation limit, `mailbox-passport-deny` and `-leases`, and `roymctl mailbox passport`.

## Close-out

Living docs to update when this ships:

- [system-architecture.md](../../../system-architecture.md): config table (`access`), coordinator endpoints, security section.
- [system-requirements-spec.md](../../../system-requirements-spec.md): Relay section.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "passport", "child token" and "lease".
- A user-facing note for relay operators, in the developer guide.

The backlog rows "Home-relay passport for the WebRTC signalling server" and "Operator passport: per-passport quotas and payment" are added with this change.
