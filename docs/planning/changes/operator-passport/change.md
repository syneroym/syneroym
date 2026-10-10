---
status: draft
living-docs-touched:
---
# Operator passport

*Reader: developers and operators.*

## Problem

Some infrastructure services are not open to everyone. An operator runs a private or paid Iroh relay, or a dedicated mailbox. Today the only gate is a fixed list in the config (`[roles.coordinator] access` lists Iroh endpoint ids). The list is read once at startup. To admit a new user the operator must edit the config and restart.

This change adds a **passport**: a signed, time-limited token. The operator gives it to a user outside the system (email, chat). The user gives it to their own substrate with `roymctl`. The substrate shows it to the operator's service and receives a **lease**: access for a limited time, which it renews.

The passport is one mechanism with two users:

| Use | Operator | Passport audience | Specified in |
| --- | --- | --- | --- |
| Home relay (this doc) | Owner of a coordinator | Substrate owner DID | [Use: home relay](#use-home-relay) |
| Dedicated mailbox | Owner of a mailbox | Service owner DID | [service-mailbox](../service-mailbox/change.md) |

This doc specifies the shared core and the home-relay use. The mailbox doc specifies the mailbox use. It adds only its own ability, caveats and quotas.

Open services do not need a passport. Open relays and open mailboxes are listed by a community registry ([registry-listed-coordinators](../registry-listed-coordinators/change.md), [service-mailbox](../service-mailbox/change.md)). A service that needs a passport is never listed. The user learns about it directly and configures it by hand.

## Scope and non-goals

- In scope:
  - **Core:** the token format, the challenge-and-register flow, the lease table, the deny list, the passport verification code and the `roymctl` command that issues a passport.
  - **Home relay:** a registration endpoint on the coordinator, a dynamic allow-list for the Iroh relay, the substrate side (store, register, renew, warn) and the `roymctl` commands to give a passport to a substrate.
  - Admission only. A user with a lease has the same limits as any other user.
- Not in scope:
  - The WebRTC signalling server. It has no access control today (deferred backlog).
  - Per-passport quotas on connections or bandwidth.
  - Payment.
  - The mailbox use. It is specified in [service-mailbox](../service-mailbox/change.md), built on the core written here.

## Acceptance criteria

Core. Each passport use must pass these in its own tests.

| ID | Criterion | Test |
| --- | --- | --- |
| P-1 | The operator can issue a passport with an audience (owner DID), an ability, a resource, an expiry and use-specific caveats. | |
| P-2 | Registration is refused when: the chain does not end at the operator, the token is expired, the ability or resource does not match, the audience does not match the owner in the delegation certificate, the proof-of-possession signature is wrong, the challenge is old or already used, or a count limit is used up. Each case has its own error. | |
| P-3 | A leaked passport is not enough. The caller must also hold a delegation certificate from the audience and the key that signs the challenge. | |
| P-4 | Access ends when the passport expires, unless the user registered again with a valid passport. The lease never outlasts the token or the operator limit `max_lease_secs`. | |
| P-5 | After the operator's service restarts, users with a running lease keep their access. | |
| P-6 | The operator can deny a passport by its id. Access ends at the next check. | |
| P-7 | The registration endpoints limit request size and the work done before a request is verified. An unauthenticated caller cannot use them to exhaust the service. | |
| P-8 | `roymctl` can issue a passport. | |

Home relay

| ID | Criterion | Test |
| --- | --- | --- |
| H-1 | A substrate with a valid passport can register. After that the relay lets its node id connect, with no restart. | |
| H-2 | `max_nodes` limits how many nodes use one passport. | |
| H-3 | An open relay (`access = "everyone"`) refuses registration with a clear error. | |
| H-4 | The substrate registers again before the lease ends, retries with backoff, and logs a warning and sets a metric when expiry is near. | |
| H-5 | `roymctl` can hand a passport to a substrate and show its lease. | |

## Design

### Core

**Token.** A `CapabilityToken` ([ADR-0015](../../../decisions/0015-ucan-capability-model.md), `crates/ucan`). Each use fills in the same fields:

| Field | Meaning |
| --- | --- |
| issuer | The operator DID, directly or at the end of a delegation chain. |
| audience | The owner DID of the user. It is the owner and not one device, because an owner has several. |
| capability | An ability on a resource that names the operator's service. |
| caveats | `expires_at` and use-specific limits. |

**Registration.** Two steps, so a request cannot be replayed:

1. `challenge` returns a random value. It is kept in memory for a short time and works once.
2. `register` carries the passport, a `DelegationCertificate` from the audience to the key of the caller, and that key's signature over the challenge.

The operator's service then:

1. Reads the challenge and removes it.
2. Verifies the signature of the caller key (proof of possession).
3. Verifies the delegation certificate (audience to caller key).
4. Verifies the token chain with `verify_chain`. The root must be the operator. It checks the ability, the resource, the expiry and the audience.
5. Checks that the token id is not denied, and any count caveat.
6. Writes one lease row `(token_id, audience, caller_key, lease_ends_at)`. The lease ends at the earlier of the token expiry and `max_lease_secs`.

The response gives `lease_ends_at`. The service allows a caller when it has a row with a lease still running. It reads from memory, filled from SQLite at startup. Registration must be over TLS, because the passport is sensitive.

**What a use provides.** The ability and resource names, its caveats, which key signs the challenge, and how the lease is enforced (relay: the access function; mailbox: the quota lookup).

**Code.** The verification, the challenge store, the lease table and the deny list are one reusable module, not a copy for each use. *Open:* which crate holds it (`crates/ucan` or a new small crate).

**Rejected options.**
- **One-time registration with no expiry of access.** The token would give access forever. The token expiry must bound access.
- **A token tied to one device.** Owners run several devices, and a device key can change.
- **Registration by `roymctl` directly.** The service must see the caller's own key signature, so the substrate must make the call.
- **Listing passport services in the registry.** The registry lists open services only.

### Use: home relay

**Ability and caveats.** Ability `relay/home` on a resource that names the coordinator. Caveats: `expires_at`, `max_nodes`. The caller key is the substrate's node key.

**Endpoints.** On the coordinator's info HTTP server (the one with `/v1/info`): `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.

**Access setting.** A new value for `[roles.coordinator] access`: `"passport"`. Only nodes with a running lease are admitted. `"everyone"` and the static list keep their meaning. *Open question below:* whether a static list and passports may be used together.

**Substrate.** The passport is kept by the substrate and used for the coordinator named by `parent_coordinator.iroh.url`. The substrate registers at start and again when about a third of the lease is left. Failures are retried with backoff. A warning and a metric start when less than a set time is left (suggest 7 days).

**roymctl.** These names are a proposal. Check them against the existing `roymctl` layout.

- `roymctl coordinator passport issue --audience <did> --expires-in <duration> --max-nodes <n>` writes a passport file. Run by the coordinator owner. The mailbox use gets its own `issue` variant on the same code.
- `roymctl substrate home-relay set --coordinator <url> --passport <file>` hands it to a substrate through the control plane. The substrate registers itself.
- `roymctl substrate home-relay status` shows the lease and the expiry.

## Open questions

- *Unverified:* does a coordinator have an owner DID today? The `ControllerAgreement` claim step names an owner for a substrate node. Check that a coordinator role can use it as the token root. The same question applies to a mailbox.
- How is the resource named (for example a URI that holds the operator's owner DID and the service id)? Check the `ResourceUri` forms in ADR-0015. New abilities (`relay/home`, `mailbox/use`) may need an addition to that ADR's vocabulary.
- May `access` combine a static list with passports? Suggest: yes, allowed when either matches. Needs a config shape.
- Where does the substrate keep the passport: a state store set by `roymctl` (preferred, no restart), or a file path in the config, like `resolve_ucan` in `crates/core/src/config/roles.rs`?
- A suitable default for `max_lease_secs`.

## Tasks

- [ ] Resolve the open questions. Amend ADR-0015 for the new abilities if needed.
- [ ] Core module: passport issue and verify, challenge store, lease table, deny list. Tests for each error in P-2.
- [ ] Coordinator: challenge and register endpoints, dynamic access function.
- [ ] Substrate: store passport, register, renew, warn.
- [ ] `roymctl` commands.
- [ ] Integration test: restricted relay, registration, expiry, restart, deny. Use `common::alloc_ports`.
- [ ] Update the living docs listed below.

## Deviations

## Close-out

Living docs to update when this ships:

- [system-architecture.md](../../../system-architecture.md): config table (`access`), coordinator endpoints, security section.
- [system-requirements-spec.md](../../../system-requirements-spec.md): Relay section.
- A user-facing note for relay operators, in the developer guide.

The backlog row "Home-relay passport for the WebRTC signalling server" is already added.
