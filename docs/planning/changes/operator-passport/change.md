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
| P-1 | The operator can issue a passport with an audience (owner DID), an ability, a resource, an expiry and use limits. | |
| P-2 | Registration is refused when: the chain does not end at an accepted operator root; the token is expired; the ability or resource does not match; the proven key is not bound to the subject (for the relay the two are the same key; for the mailbox the instance key is bound to the member master by its service-instance certificate); the proof of possession fails (challenge form: the signature is wrong, or the challenge is old or already used); or a use limit is reached. Each case has its own error. | |
| P-3 | A leaked passport is not enough. The caller must also prove that it holds the key bound to the subject of the owner's child token: by a challenge signature (relay) or by its transport identity (mailbox). | |
| P-4 | A caller whose lease has ended is refused when it connects the next time. A connection that is already open is not cut. The lease never outlasts the token or the `max_lease_secs` of its use. | |
| P-5 | After the operator's service restarts, users with a running lease keep their access. | |
| P-6 | The operator can deny a passport by its id. A denied caller is refused when it connects the next time. | |
| P-7 | Challenge form only. The registration endpoints accept a body of at most 16 KiB, keep at most 1,000 challenges (the oldest is dropped first), give each challenge a life of 60 seconds, do the size and shape checks before any signature check, and limit each source address to 10 requests per minute. | |
| P-8 | `roymctl` can issue a passport, deny a passport by id, and list the running leases. | |
| P-9 | Challenge form only. The challenge signature covers a domain tag, the node DID of the host substrate of the service being registered with, the challenge, and a hash of the token. A signature made for one service is refused by another. | |
| P-10 | Challenge form only. The registration endpoints are served only over TLS. A service with passport access and no TLS config stops at startup. | |

Home relay

| ID | Criterion | Test |
| --- | --- | --- |
| H-1 | A substrate with a valid passport can register. After that the relay lets its node id connect, with no restart. | |
| H-2 | `max_nodes` limits how many distinct nodes use one passport. | |
| H-3 | An open relay (`access = "everyone"`) refuses registration with a clear error. | |
| H-4 | The substrate registers again when a third of the lease is left, retries with backoff, and logs a warning and sets a metric when 7 days or less of the passport life is left. | |
| H-5 | `roymctl` can hand a passport to a substrate and show its lease. | |
| H-6 | An unknown value of `[roles.coordinator] access` stops the coordinator at startup. Today any unknown string, including a typo, opens the relay to everyone. | |
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

**Operator root.** The root of the chain is the controller of the substrate that hosts the service. If that substrate has no controller, it is the node DID. The service accepts a passport rooted at either one. A coordinator role has no stable DID of its own: it makes a new random Iroh key at every start (`crates/coordinator_iroh/src/coordinator.rs:193-200`). Passports stop working if the `ControllerAgreement` of the host substrate expires or changes.

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

**Substrate.** The passport is kept by the substrate and used for the coordinator named by `parent_coordinator.iroh.url`. The substrate registers at start, before the endpoint first connects to the relay (otherwise the reconnect backoff of Iroh delays access after registration), and again when a third of the lease is left. Failures are retried with backoff.

**roymctl.** These names are a proposal. Check them against the existing `roymctl` layout.

- `roymctl coordinator passport issue --audience <did> --expires-in <duration> --max-nodes <n>` writes a passport file. Run by the operator. The mailbox use gets its own `issue` variant on the same code.
- `roymctl coordinator passport deny <passport-id>` and `roymctl coordinator passport leases` deny a passport and list the running leases.
- `roymctl substrate home-relay set --coordinator <url> --passport <file>` runs on the owner's machine. It signs the child token for the node key of the substrate with the owner key, and hands it to the substrate through the control plane. The substrate registers itself.
- `roymctl substrate home-relay status` shows the lease and the expiry.

## Open questions

None open. Decided 2026-10-11:

- The shared core is the new crate `crates/passport` (see Design, Code).
- `access` may combine a static list with passports (see Access setting).
- The substrate keeps the child token in a state store set by `roymctl` through the control plane. No restart and no config path.

## Tasks

- [x] Amend ADR-0015: abilities `relay/home` and `mailbox/use`, the selector resource form, reserved facts keys (A10).
- [ ] Core module: passport issue, child token, verify, both proof forms, signer binding, challenge store, lease table keyed on subject, deny list. Tests for each error in P-2 (both proof forms) and for P-9.
- [ ] Coordinator: challenge and register endpoints, dynamic access function, strict `access` parsing.
- [ ] Substrate: store the child token, register before first connect, renew, warn.
- [ ] `roymctl` commands (issue, deny, leases, home-relay set and status).
- [ ] Integration test: restricted relay, registration, expiry, restart, deny, reconnect after expiry. Use `common::alloc_ports`.
- [ ] Update the living docs listed below.

## Deviations

## Close-out

Living docs to update when this ships:

- [system-architecture.md](../../../system-architecture.md): config table (`access`), coordinator endpoints, security section.
- [system-requirements-spec.md](../../../system-requirements-spec.md): Relay section.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "passport", "child token" and "lease".
- A user-facing note for relay operators, in the developer guide.

The backlog rows "Home-relay passport for the WebRTC signalling server" and "Operator passport: per-passport quotas and payment" are added with this change.
