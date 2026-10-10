---
status: draft
living-docs-touched:
---
# Operator passport

*Reader: developers and operators.*

## Problem

Some infrastructure services are not open to everyone. An operator runs a private or paid Iroh relay, or a dedicated mailbox. Today the only gate is a fixed list in the config (`[roles.coordinator] access` lists Iroh endpoint ids). The list is read once at startup. To admit a new user the operator must edit the config and restart.

This change adds a **passport**: a signed, time-limited token. The operator gives it to a user outside the system (email, chat). The user's owner key re-delegates it to the key of their own substrate or service, with `roymctl`. The substrate shows the result to the operator's service and receives a **lease**: access for a limited time, which it renews.

The passport is one mechanism with two users:

| Use | Operator | Passport audience | Key that signs the challenge | Specified in |
| --- | --- | --- | --- | --- |
| Home relay (this doc) | Controller of the coordinator's host substrate | Substrate owner DID | The substrate node key | [Use: home relay](#use-home-relay) |
| Dedicated mailbox | Controller of the mailbox's host substrate | Service owner DID | The service instance key | [service-mailbox](../service-mailbox/change.md) |

This doc specifies the shared core and the home-relay use. The mailbox doc specifies the mailbox use. It adds only its own ability, limits and quotas.

Open services do not need a passport. Open relays and open mailboxes are listed by a community registry ([registry-listed-coordinators](../registry-listed-coordinators/change.md), [service-mailbox](../service-mailbox/change.md)). A service that needs a passport is never listed. The user learns about it directly and configures it by hand.

## Scope and non-goals

- In scope:
  - **Core:** the token format, the challenge-and-register flow, the lease table, the deny list, the verification code, and the `roymctl` commands that issue, deny and list passports.
  - **Home relay:** a registration endpoint on the coordinator, a dynamic allow-list for the Iroh relay, the substrate side (store, register, renew, warn), and the `roymctl` commands to give a passport to a substrate.
  - An amendment to ADR-0015 for the new abilities and the resource form (see Design).
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
| P-2 | Registration is refused when: the chain does not end at an accepted operator root; the token is expired; the ability or resource does not match; the key that signs is not bound to the final audience (for the relay the two are the same key; for the mailbox the instance key is bound to the member master by its service-instance certificate); the proof-of-possession signature is wrong; the challenge is old or already used; or a use limit is reached. Each case has its own error. | |
| P-3 | A leaked passport is not enough. The caller must also hold the key bound to the audience of the owner's child token, and sign the challenge with it. | |
| P-4 | A caller whose lease has ended is refused when it connects the next time. A connection that is already open is not cut. The lease never outlasts the token or `max_lease_secs`. | |
| P-5 | After the operator's service restarts, users with a running lease keep their access. | |
| P-6 | The operator can deny a passport by its id. A denied caller is refused when it connects the next time. | |
| P-7 | The registration endpoints accept a body of at most 16 KiB, keep at most 1,000 challenges (the oldest is dropped first), give each challenge a life of 60 seconds, do the size and shape checks before any signature check, and limit each source address to 10 requests per minute. | |
| P-8 | `roymctl` can issue a passport, deny a passport by id, and list the running leases. | |
| P-9 | The challenge signature covers a domain tag, the node DID of the host substrate of the service being registered with, the challenge, and a hash of the token. A signature made for one service is refused by another. | |
| P-10 | The registration endpoints are served only over TLS. A service with passport access and no TLS config stops at startup. | |

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
2. The **child token**: the owner re-delegates the passport to the key of the caller. Issuer is the owner DID, audience is the caller key, `proofs` holds the passport. It has the same ability and resource, an expiry no later than the passport, and use limits no wider than the passport.

`verify_chain` already checks that each audience is the next issuer. The service calls it with the caller key as the expected audience. No `DelegationCertificate` and no `ControllerAgreement` is part of this flow. The owner is only the middle link of the chain, and the owner key signs the child token when `roymctl` hands the passport over.

| Field | Value |
| --- | --- |
| capability | One ability on one resource that names the operator's service. |
| facts | The use limits, for example `{"max_nodes": 3}`. They are signed with the token. |
| expires | `expires_at_secs`. |

**Use limits are signed facts.** ADR-0015 A3 allows only the caveats `where`, `fields` and `can_delegate`, and a caveat must never need a data lookup. A count such as `max_nodes` needs one. So the limits go in the signed `facts`, and the **operator's service** checks them, not `verify_chain`. The ADR amendment records this.

**Passport id.** `token_id` is the SHA-256 of the canonical signed body of the passport (the body that its signature covers). `CapabilityToken` has no id field today. The deny list and the use counts use this id.

**Resource name.** A bare `substrate:<did>` matches every resource (`crates/ucan/src/capability.rs`). Use a selector form instead: `substrate:<host node did>/relay` for the relay and `substrate:<host node did>/mailbox` for the mailbox. The DID is always the node DID of the host substrate, whichever root (controller or node DID) issued the passport. The challenge signature (P-9) names the same node DID.

**ADR-0015 amendment.** Required, not optional. It adds the abilities `relay/home` and `mailbox/use` (the ability set is closed, A2), the selector resource form above, and the reserved facts keys for use limits.

**Operator root.** The root of the chain is the controller of the substrate that hosts the service. If that substrate has no controller, it is the node DID. The service accepts a passport rooted at either one. A coordinator role has no stable DID of its own: it makes a new random Iroh key at every start (`crates/coordinator_iroh/src/coordinator.rs:193-200`). Passports stop working if the `ControllerAgreement` of the host substrate expires or changes.

**Registration.** Two steps, so a request cannot be replayed:

1. `challenge` returns a random value. It is kept in memory for 60 seconds and works once.
2. `register` carries the child token and the caller's signature over `(domain tag, DID of this service, challenge, hash of the child token)`.

The operator's service then:

1. Reads the challenge and removes it.
2. Verifies the caller's signature against the audience of the child token.
3. Verifies the chain with `verify_chain`, with the caller key as expected audience and the operator root as an accepted root. It checks the ability, the resource and the expiry.
4. Checks the use limits and that the passport id is not denied. The limits are read from the **passport** (the root token) only. A limit in the child token is treated as narrower, or ignored: `verify_chain` does not compare facts between tokens, so an owner could otherwise write `max_nodes: 1000` under a passport that says 3.
5. Writes one lease row `(passport_id, owner_did, caller_key, lease_ends_at)`. The lease ends at the earliest of the child token expiry and `max_lease_secs`. Default `max_lease_secs` is 24 hours.

The response gives `lease_ends_at`. The service allows a caller when it has a row with a lease still running. It reads from memory, filled from SQLite at startup. Registration must be over TLS (P-10), because the token is sensitive. The info server runs TLS only when `[tls]` is set, so passport access requires it.

**What a use provides.** The ability and resource names, its facts, which key signs the challenge, and how the lease is enforced (relay: the access function; mailbox: the quota lookup). For a use where the key that signs differs from the audience of the child token, it also says how they are bound. The mailbox use binds an instance key to a stable service master by the existing service-instance certificate that the transport already checks.

**Code.** The verification, the challenge store, the lease table and the deny list are one reusable module, not a copy for each use. *Open:* which crate holds it (`crates/ucan` or a new small crate).

**Rejected options.**
- **One-time registration with no expiry of access.** The token would give access forever. The token expiry must bound access.
- **A token tied to one device.** Owners run several devices, and a device key can change.
- **A `DelegationCertificate` or `ControllerAgreement` as the owner-to-key proof.** For a substrate the link to its owner is a `ControllerAgreement` (`crates/identity/src/substrate.rs:48`), which is not a certificate. The certificate scopes are only `routing`, `session-auth`, `service-instance` and `record-signing`. For a service, no signed owner-to-master link exists; the owner is only a local record (`crates/core/src/local_registry.rs:314`). A child token needs none of these.
- **Registration by `roymctl` directly.** The service must see the caller's own key signature, so the substrate must make the call.
- **Listing passport services in the registry.** The registry lists open services only.

### Use: home relay

**Ability and facts.** Ability `relay/home` on `substrate:<host node did>/relay`. Facts: `max_nodes`. The caller key is the substrate's node key, and it is also the audience of the child token.

**Endpoints.** On the coordinator's info HTTP server (the one with `/v1/info`): `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.

**Access setting.** A new value for `[roles.coordinator] access`: `"passport"`. Only nodes with a running lease are admitted. `"everyone"` and the static list keep their meaning. Any other value stops the coordinator at startup (H-6).

**When access ends.** The Iroh relay checks access only when it accepts a connection (`iroh-relay-0.97.0/src/server/http_server.rs:750`), and no API to drop a connected client was found. So an expired lease or a denied passport ends access at the next reconnect. A live connection is not cut.

**Substrate.** The passport is kept by the substrate and used for the coordinator named by `parent_coordinator.iroh.url`. The substrate registers at start, before the endpoint first connects to the relay (otherwise the reconnect backoff of Iroh delays access after registration), and again when a third of the lease is left. Failures are retried with backoff.

**roymctl.** These names are a proposal. Check them against the existing `roymctl` layout.

- `roymctl coordinator passport issue --audience <did> --expires-in <duration> --max-nodes <n>` writes a passport file. Run by the operator. The mailbox use gets its own `issue` variant on the same code.
- `roymctl coordinator passport deny <passport-id>` and `roymctl coordinator passport leases` deny a passport and list the running leases.
- `roymctl substrate home-relay set --coordinator <url> --passport <file>` runs on the owner's machine. It signs the child token for the node key of the substrate with the owner key, and hands it to the substrate through the control plane. The substrate registers itself.
- `roymctl substrate home-relay status` shows the lease and the expiry.

## Open questions

- Which crate holds the shared core module?
- May `access` combine a static list with passports? Suggest: yes, allowed when either matches. Needs a config shape.
- Where does the substrate keep the child token: a state store set by `roymctl` (preferred, no restart), or a file path in the config, like `resolve_ucan` in `crates/core/src/config/roles.rs`?

## Tasks

- [ ] Amend ADR-0015: abilities `relay/home` and `mailbox/use`, the selector resource form, reserved facts keys.
- [ ] Core module: passport issue, child token, verify, challenge store, lease table, deny list. Tests for each error in P-2 and for P-9.
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
