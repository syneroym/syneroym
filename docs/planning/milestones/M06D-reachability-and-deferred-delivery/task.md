# M06D: Reachability and deferred delivery

*Reader: developers who build these three changes.*

**Status: in progress (2026-10-13).** S0 and A1 to A4 are done. Progress is in [status.md](status.md).
This milestone builds three features: registry-listed relays (a substrate finds
an open home relay through its registry), the operator passport (a signed token
that admits a user to a private relay or mailbox), and the service mailbox (a
service that holds sealed messages for a sleeping receiver). This plan orders
the work of their three change docs. It does not repeat their requirements or
design. Read them first:

- [registry-listed-coordinators](../../changes/registry-listed-coordinators/change.md) (criteria `AC-*`)
- [operator-passport](../../changes/operator-passport/change.md) (criteria `P-*`, `H-*`)
- [service-mailbox](../../changes/service-mailbox/change.md) (criteria `M-*`, `K-*`, `S-*`, `R-*`, `D-*`), with [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md)

The goal of the order: write each shared part once, in its final form, before
the second user of that part needs it. Each slice below is one PR (or a small
group of PRs) that passes `mise run verify` on its own.

## 1. Shared parts

These are the parts that more than one change uses. A wrong order here causes
rework.

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| X1 | **Home-relay handle.** The router builds the Iroh endpoint with an empty relay map and exposes a handle that inserts and removes the home relay at run time. The record publisher reads the relay URL from a `watch` channel and publishes again when it changes. | Coordinators (all resolve paths, `AC-8`, `AC-11`). Passport (`H-7`: register *before* the relay is inserted). | A1 (Done) |
| X2 | **Curated-list framework in the registry.** A config list, entry checks (normalised id, no duplicates), a `GET` route with `max_age_secs`, and parent forwarding with a hop limit. | Coordinators (`AC-3`, `AC-13`, `AC-15`). Mailboxes (`GET /mailboxes`). | A2 (Done) |
| X3 | **Rendezvous ranking helper and URL normaliser** (`SHA-256` over length-prefixed fields), in `crates/core`. | Substrate relay choice (`AC-4`). `roymctl ... --mailbox auto` (`K-3`). | A2 (Done) |
| X4 | **Passport core module.** Token id, issue, child token, chain check with limits read from the root only, deny list, lease table, distinct-subject counting, challenge store, request limits. | Relay (`H-*`). Dedicated mailbox (`D-*`). | B1 |
| X5 | **ADR-0015 amendment.** Abilities `relay/home` and `mailbox/use`, the selector resource form, and *all* reserved facts keys (`max_nodes`, `max_services`, `max_bytes`, `max_messages`, `max_ttl_secs`). | Both passport uses. | S0 |
| X6 | **Parked state in `async_queue`.** A row outside the attempt budget, with its own poll interval and deadline. The supervisor keeps its behaviour. | Proxy outbox and conversation outbox (`S-6`, `S-9`). | C1 |
| X7 | **Coordinator info server** (`/v1/info`). Relay client count and cached registry lookup first. Passport routes and the TLS rule later. | Coordinators (`AC-6`, `AC-17`). Passport (`/v1/home-relay/*`, `P-10`). | A3, then B2 |

Facts checked in the code for this plan:

- After A1 the publisher reads a `watch` channel and wakes on `relay_rx.changed()` (`crates/substrate/src/runtime/publish.rs:100`).
- The registry already accepts a byte-identical record with the same timestamp as a refresh, and rejects an older timestamp (`crates/community_registry/src/registry.rs:243-299`, `admit_endpoint`). So the registry side of `K-4` is mostly tests. *Unverified:* the same rule on the DHT path.
- A1 made the `access` parse strict (`crates/core/src/config/access.rs`). Both the strict parse (`H-6`) and the passport access function replace the earlier `match`.
- `InfoState.substrate_id` (`crates/coordinator_iroh/src/info_endpoint.rs`) is the coordinator's random endpoint ID, not the host node ID. B2 must pass the host substrate's node DID and controller into the info server when building passport routes.
- X25519 is already in the build: `x25519-dalek` 2.0 comes in through `vodozemac`, which the conversation capability uses. It is not a direct workspace dependency. The router stream handshake uses `p256` ECDH and `aes-gcm` (`crates/router/src/route_handler/encryption.rs`). No crate in the workspace gives a sealed box.
- `crates/ucan` depends only on `crates/identity`. `crates/community_registry`, `crates/router`, `crates/sdk` and `apps/roymctl` all depend on `crates/core`, so X3 in `crates/core` reaches every user.

## 2. Gaps in the change docs, and the decisions

These change the shape of a shared part. All six are decided (2026-10-11) and written into the change docs.

1. **The mailbox has no `challenge` method.** The passport core uses a two-step challenge and register (`P-2`, `P-7`, `P-9`). The mailbox doc says the instance key "signs the challenge" (`D-2`), but its method list (`M-1`) has no `challenge`. The mailbox `register` runs over Iroh, where the transport already proves the caller's instance key (ADR-0016, `M-2`). **Decided:** X4 takes the proof of possession as an input with two forms: a challenge signature (relay, over HTTPS) or a transport-verified key (mailbox). The mailbox then needs no challenge, and `P-7`, `P-9` and `P-10` apply to the relay use only. Write this in both docs.
2. **Lease key.** The passport lease row has `caller_key`. For the relay this is the node key. For the mailbox the stable subject is the member master DID, because instance keys change at relocation and `max_services` counts masters (`D-3`). **Decided:** key the lease table and the distinct-subject count on the audience of the child token ("subject"). For the relay, subject and signing key are the same key.
3. **Signer binding.** The relay signer *is* the subject. The mailbox signer is bound to the subject by the instance certificate. X4 needs a binding hook from the start, tested with both forms, or the mailbox use must change its interface later.
4. **`access` config shape.** The passport doc asks whether a static list and passports may combine. Decide now: A1 writes the strict parser, and B2 must not change its type again. **Decided:** yes, either match admits; shape `access = "everyone" | "passport" | [ids]`, plus `access = { list = [ids], passport = true }` for both.
5. **Mailbox key algorithm. Decided: X25519, sealed with HPKE** ([RFC 9180](https://www.rfc-editor.org/rfc/rfc9180), base mode, `DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256`, `AES-256-GCM`), using the `hpke` crate. The key goes into the signed record (`K-1`), so this is fixed before C3.
   - *Why X25519:* it is already in the build through `vodozemac`. The key is 32 bytes. The senders are always substrates (Rust), never a browser, so the P-256 reason that applies to browser clients (WebCrypto) does not apply here.
   - *Why HPKE:* "seal to a public key with a fresh key for each message" is exactly what HPKE base mode does. It is a standard with test vectors. The `info` input gives domain separation (use a fixed tag such as `syneroym-mailbox-v1`).
   - *Rejected: our own X25519 + HKDF + AES-GCM.* It needs no new crate, but it is our own composition. We would have to bind both public keys into the key derivation ourselves, and we would have no outside test vectors.
   - *Rejected: P-256 ECDH + AES-GCM like the router.* The router code is an interactive stream handshake, not a sealed box, so little of it can be reused. P-256 gives no benefit here.
   - *Check in S0:* the `curve25519-dalek` and `rand_core` versions that `hpke` needs fit the workspace pins (iroh uses a pre-release crypto chain). If they do not, fall back to the rejected own composition, with RFC 9180 test vectors adapted.
6. **Dedicated mailbox lease while asleep. Decided: the lease limit depends on the use.** `max_lease_secs` is a setting for each use, not one 24-hour value. Relay: 24 hours. Mailbox: 30 days, the same as the queue lifetime. A mailbox lease still never outlasts the child token or the passport. The receiver renews it each time it registers. So a queue keeps its raised quota while it lives, and messages stay as long as the quota is not exceeded. The passport doc's "default 24 hours" changes to "relay default 24 hours".

## 3. Slices

Sizes are counted in the project's AI-assisted workflow (plan session, implementation session, review by the other tool, fix session), measured on the M06C slices: a slice usually went from plan to merge in 1 to 3 calendar days.

- S: about 1 day, one PR.
- M: 2 to 3 days, one PR.
- L: 4 to 5 days. Consider two PRs.

### S0 — Decisions and spike (docs only, S)

- **Spike:** on iroh 0.97, build an endpoint with `RelayMode::Custom(RelayMap::empty())`, then `insert_relay`. Confirm it becomes the home relay and that `remove_relay` switches cleanly. Record the result in the coordinators doc (its first task). If this fails, X1 needs another design, and A1 and B3 both change.
- Done: write the six decisions of §2 into the change docs.
- Done: write the ADR-0015 amendment (X5) for both uses at once ([A10](../../../decisions/0015-ucan-capability-model.md#a10-operator-passports-two-abilities-two-selectors-signed-use-limits)).
- Done: decide the crate homes (written into the change docs):
  - X3 (ranking, normaliser, list wire types): `crates/core`.
  - X4 (passport core): a new small crate `crates/passport` (`syneroym-passport`) on top of `crates/ucan`. It needs SQLite and a request limiter, which `crates/ucan` should not carry.
  - The coordinator resolver task: `crates/substrate`. It needs the endpoint handle and the record publisher, which live there.
- Moved out of S0: the two mailbox open questions that fix a format need a code study. They are now the first step of the slice that needs each one: the deploy-call field for the mailbox private key opens C3, and where a method declares mailbox eligibility opens C5. Neither format is used before its slice, so moving them causes no rework.

### Track A — Registry-listed coordinators

**A1 — Relay plumbing (M, Done).** No new user feature yet. It makes the base that A4 and B3 build on.
- X1: home-relay handle. A fixed `url` goes through the same handle (insert at boot), so there is one code path. Define the source as an enum, for example `HomeRelaySource::{Fixed { url }, Registry}`. A4 adds the registry source, and B3 adds a passport step to `Fixed`.
- Remove both n0 fallbacks; use the empty relay map (`AC-10`).
- `parent_coordinator.iroh.url` becomes optional; write the dev profile relay explicitly; check the test configs that rely on a missing section (`AC-1`, `AC-14`, `AC-16`).
- Remove `coordinator_discovery_url`.
- Strict `access` parsing with the final shape from §2.4 (`H-6`). This is pulled forward from the passport change, because it is a safety fix on the same `match`, and the registry list assumes `access = "everyone"` is exact.

**A2 — Curated lists and ranking (M, Done).** Parallel with A3.
- X2 in `crates/community_registry`, generic over the entry type. The coordinator list is its first instance: `[[roles.community_registry.coordinators]]`, `GET /coordinators?transport=iroh-relay`, `max_age_secs`, checks at startup, parent forwarding with a hop counter (`AC-3`, `AC-13`, `AC-15`).
- X3 in `crates/core`, with a fixed test vector so the ranking never changes between compiler versions (`AC-4`).
- Done when adding a second list needs only a config field, an entry type and one route. C7 relies on this.

**A3 — Coordinator info (S, Done).** Parallel with A2.
- Relay client count from `accepts` minus `disconnects`; `at_capacity` from that count (`AC-6`, server side).
- Cache the registry lookup in `/v1/info` (`AC-17`).

**A4 — Substrate resolver (M, Done).** Needs A1, A2, A3.
- Background resolve, `coordinators.json` cache under `storage.db_dir`, probe `info_url`, keep the saved choice, fail over after 3 failures in 5 minutes, degraded mode with backoff, publish again on change (`AC-2`, `AC-5` to `AC-9`, `AC-11`, `AC-12`).
- Integration tests from the coordinators doc, with `common::alloc_ports`.
- Close out the coordinators change doc and its living docs.
- Done 2026-10-13. Built as planned. The differences and the decisions `D-A4-1` to `D-A4-19` are in the `## Deviations` of the [coordinators change doc](../../changes/registry-listed-coordinators/change.md). B3 must not set the relay by hand on a node whose source is `Registry` or none (`D-A4-15`, written into the passport doc).

### Track B — Operator passport

**B1 — Passport core (M).** Needs S0 only. It can run in parallel with A1 to A3.
- X4 with the shape from §2.1 to §2.3: proof of possession as an input, lease keyed on subject, binding hook. Limits read from the passport only.
- Tests for each `P-2` error, `P-3`, `P-9`, and the `P-7` limits, with both proof forms and both binding forms.
- `roymctl coordinator passport issue` (an offline signature; one code path with an ability parameter, so the mailbox `issue` variant is a thin wrapper).

**B2 — Relay use, coordinator side (M).** Needs A1, A3 (same info server) and B1.
- `access = "passport"` (and the combined form); a run-time access function that reads the lease table.
- `/v1/home-relay/challenge` and `/register` on the info server; refuse to start with passport access and no TLS (`P-10`). The test harness running the info server with TLS is verified by `crates/coordinator_iroh/tests/tls_rotation.rs`.
- `/v1/info` `identity` reports `null` until B2 passes the host substrate's node DID and controller into the info server.
- `spawn_http_info_server` takes a ready `axum::Router`; B2 merges its passport routes into it.
- Decide whether the coordinator's own endpoint is admitted on its own relay with passport access.
- Leases survive a restart; deny by id; `roymctl coordinator passport deny` and `leases` (`P-4` to `P-6`, `P-8`, `H-1` to `H-3`).

**B3 — Relay use, substrate side (M).** Needs A1 and B2. It does not need A4.
- Child-token store set through the control plane; `roymctl substrate home-relay set` and `status`.
- In the `Fixed` source of X1: register, then insert the relay (`H-7`). Renew when a third of the lease is left, back off, warn and set a metric at 7 days of passport life left (`H-4`, `H-5`).
- Integration test: restricted relay, register, expiry, restart, deny, reconnect after expiry.
- Close out the passport change doc. The mailbox use stays open in the mailbox doc.

### Track C — Service mailbox

**C1 — Parked queue state (M).** Needs nothing. It can start first.
- X6 in `crates/async_queue`. Supervisor tests stay green. No outbox uses it yet; C5 does.

**C2 — Sealed message and receipt formats (S).** Needs the key decision in S0.
- Outer and inner parts, sender signature over destination, idempotency key, payload hash and send time; signed `delivered` receipt (`M-13`).
- Tests: tamper, wrong key, replay, old send time.

**C3 — Record, keys and lookup (L).** Needs C2.
- First: decide which part of the control-plane deploy call carries the mailbox private key, and where the instance keeps it. Design the field with room for the owner's child token, which C8 adds.
- `EndpointInfo.mailbox` (`K-1`). Mailbox key made with the member master, delivered with the instance certificate, carried at relocation (`K-2`). Per-member certificate lifetime (`K-8`).
- `roymctl svc deploy` and `app deploy` with `--mailbox <service-id>`. `auto` waits for C7.
- `resolve_iroh_addr` returns the mailbox field without following the substrate record (`K-5`). Durable last-record store on the sender (`K-6`). Private services get no field (`K-7`).

**C4 — Mailbox service (L).** Needs C2 and C3.
- `[roles.mailbox]`, native service, SQLite store, all seven methods, quotas, fair eviction, sweeps, newest-certificate rule, receipts, hourly record publish (`M-1` to `M-14`, `K-4`).
- Open limits only. `register` takes no passport yet.

**C5 — Sender path (L).** Needs C1, C3 and C4. Parallel with C6.
- First: decide where a guest method declares that it is mailbox-eligible (app manifest or WIT interface).
- Eligibility check (`S-1`), one direct attempt then deposit, `status` polling, `ProxyRouter` hook below both outboxes (`S-2` to `S-5`, `S-7`, `S-8`).
- `deposited` state in both outboxes on X6 (`S-6`, `S-9`), the `ConversationDeliveryState` value, the WIT enum in `conversation.wit`, and the Hub UI. Group pushes.

**C6 — Receiver path (L).** Needs C3 and C4. Parallel with C5.
- Pull worker, unseal, `dispatch_json_rpc_once` with the sender's identity, signed ack, waiting for a renewed certificate, `mailbox.pull-now` (`R-1` to `R-6`).
- Longer idempotency memory and a larger row limit for services with a mailbox (`R-7`).

**C7 — Open mailbox list and `auto` (S).** Needs A2 and C3.
- `[[roles.community_registry.mailboxes]]` and `GET /mailboxes` as the second instance of X2.
- `--mailbox auto` with X3.
- After C5, C6 and C7: run the full integration list from the mailbox doc and close out its living docs, except the dedicated-mailbox criteria.

**C8 — Dedicated mailbox (M).** Needs B1, C4 and C7. Last.
- `register` with a passport, proof by transport identity, binding by instance certificate, raised quotas from facts, `max_services` (`D-1` to `D-3`).
- Owner signs one child token for each member master at deploy (the deploy-call field reserved in S0).
- `roymctl` mailbox `issue` variant on B1's code.
- Close out the mailbox change doc.

## 4. Order and parallel work

| Slice | Needs | Size |
| --- | --- | --- |
| S0 | — | S |
| A1 | S0 | M |
| A2 | A1 | M |
| A3 | A1 | S |
| A4 | A1, A2, A3 | M |
| B1 | S0 | M |
| B2 | A1, A3, B1 | M |
| B3 | A1, B2 | M |
| C1 | — | M |
| C2 | S0 | S |
| C3 | C2 | L |
| C4 | C2, C3 | L |
| C5 | C1, C3, C4 | L |
| C6 | C3, C4 | L |
| C7 | A2, C3 | S |
| C8 | B1, C4, C7 | M |

With one developer, a good serial order is:
S0, A1, C1, A2, A3, B1, A4, B2, B3, C2, C3, C4, C5, C6, C7, C8.

The relay work comes first. It is smaller, it makes the base for the passport,
and it tests the curated list and the ranking helper before the mailbox reuses
them. C1 is early because it is independent and touches the supervisor, which
is cheaper to check while little else is changing.

With two developers: one takes Track A then Track B. The other takes C1, C2,
C3, C4, then joins for C5 and C6.

## 5. Where rework would come from, and how this order avoids it

| Risk | Avoided by |
| --- | --- |
| The resolver (A4) and the passport (B3) each change how the endpoint gets its relay. | X1 in A1 gives one handle and one source enum. A4 and B3 each add one variant or step. |
| The mailbox list copies the coordinator list. | X2 is generic in A2, with a "second list is one route" done rule. |
| The passport core fits the relay only and must change for the mailbox. | §2.1 to §2.3 are decided in S0 and tested in B1 with both forms. |
| Two ADR-0015 amendments. | One amendment in S0 with both abilities and all facts keys. |
| The `access` type changes twice. | Final shape decided in S0, written once in A1. |
| The signed record format changes twice. | Mailbox key type fixed in S0, before C3. |
| Each outbox builds its own "deposited" state. | X6 in C1, used by both outboxes in C5. |
| The deploy call changes twice (mailbox key, then child token). | The field is designed in S0 with room for the child token. |
| A3 and B2 both edit the info server at once. | A3 lands before B2. |

## 6. Not in this plan

The deferred-backlog rows named in each change doc stay deferred: WebRTC
signalling server lists and passports, self-registering coordinators, passport
quotas and payment, several mailboxes per service, mailbox key rotation, push
wake-up, sealed sender, and Conversation first contact with an offline peer.
