---
status: in-progress
living-docs-touched:
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/developer-guide.md
  - docs/TERMINOLOGY.md
  - docs/planning/traceability-matrix.md
  - docs/demo/demo.md
---
# Registry-listed coordinators

*Reader: developers and operators.*

## Problem

A substrate needs a home relay (an Iroh relay) to be reachable from outside its network. Today the operator must set `parent_coordinator.iroh.url` by hand. A config with no `[parent_coordinator.iroh]` section builds no Iroh endpoint at all. If the section exists without `url`, the value is `http://localhost:7964`, which is only useful for development.

The architecture doc already plans a better way: a substrate asks its registry for coordinators, picks one and caches it ([system-architecture.md](../../../system-architecture.md), "Registry Entries at Deployment" section). The config key `coordinator_discovery_url` was added for this, but no code reads it, and no code lists, selects or caches coordinators.

This change builds that for the Iroh relay. A community registry operator lists open coordinators. A substrate with no coordinator of its own uses one of them.

## Scope and non-goals

- In scope:
  - A curated list of open Iroh relays in the community registry config.
  - A registry HTTP endpoint that returns the list.
  - On the substrate: choose one relay from the list, cache the list and the choice on disk, and fail over. This runs in the background and never delays boot.
  - Change `parent_coordinator.iroh.url` to optional, with the fallback when it is not set.
  - A relay that can change while the substrate runs.
  - Remove the unused `coordinator_discovery_url` setting.
  - Remove the two places where the code falls back to the public n0 relays.
- Not in scope:
  - Coordinator roles. A coordinator role never resolves its own parent relay from the registry. Only the home relay of a substrate does.
  - Coordinators that need a passport. The registry never lists them. A substrate learns about them directly from its owner and connects with an explicit `url` ([operator-passport](../operator-passport/change.md)).
  - The WebRTC signalling server. It is unchanged in this version (deferred backlog, "Registry-listed coordinators for the WebRTC signalling server").
  - Coordinators that offer themselves to the registry, registry-side health checks, capacity or region hints, weighted choice, signed lists and DNS names for community relays (deferred backlog).
  - Open mailboxes. A second curated list for them is in [service-mailbox](../service-mailbox/change.md). It shares the ranking helper written here.
  - `SyneroymClient`. It has no home relay and does not use this feature.
  - The public n0 relays as a last resort. We do not use them.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| AC-1 | `parent_coordinator.iroh.url` is optional. When it is set, the substrate uses it and never asks the registry. | `crates/substrate/tests/home_relay_e2e.rs`, `crates/core/src/config/tests.rs` |
| AC-2 | When `url` is not set and `iroh` is in `communication_interfaces`, the substrate resolves a relay in this order: saved choice, then saved list, then the registry. | |
| AC-3 | The registry returns its configured list from `GET /coordinators?transport=iroh-relay`. Each entry has `transport`, `url`, `info_url` (where `GET /v1/info` is served, because it has its own port), and `operator`. The response says how long a client may cache it (`max_age_secs`). | `crates/community_registry/src/curated_list/tests.rs` (`registry_serves_its_configured_curated_list`, `no_parent_and_empty_list_returns_empty_list_ok`), `crates/substrate/tests/curated_list_e2e.rs` (`booted_substrate_serves_curated_coordinators_list`) |
| AC-4 | The choice is deterministic. The substrate ranks the entries by SHA-256 over a fixed byte layout (see Design) and takes the first. The same inputs always give the same result on every version of the compiler. | ranking helper only (`crates/core/src/curated_list/tests.rs`); substrate use in A4 |
| AC-5 | The substrate keeps the list and its choice on disk. It does not ask the registry again until `max_age_secs` has passed. If a refresh fails, the old list stays in use, also after it has expired. | |
| AC-6 | Before it uses an entry, the substrate reads `info_url`. It skips an entry that cannot be reached or reports `at_capacity`, and takes the next entry in rank order. `at_capacity` is computed from the number of relay clients, not router connections. The count is the `accepts` counter minus the `disconnects` counter of `iroh-relay` (`server/metrics.rs`), which has no direct gauge. | |
| AC-7 | A saved choice is kept while it works, even if a new list changes the ranking. The substrate picks again only when the choice is removed from the list, or after 3 connection failures within 5 minutes. | |
| AC-8 | When the relay changes at run time, the endpoint switches relay without a restart. The substrate then publishes its endpoint record again with the new relay URL. | |
| AC-9 | The substrate is in degraded mode only when all three are true: there is no `url`, there is no saved choice or list, and there is no `registry_url` or the registry cannot be reached. In degraded mode the endpoint has an empty relay map (`RelayMode::Custom(RelayMap::empty())`), the log has a clear warning, and the substrate retries the registry with backoff when `registry_url` is set. | |
| AC-10 | The endpoint is never built with the public n0 preset, with or without a relay URL, and also when a relay URL does not parse. This covers every runtime caller of `net_iroh::build_iroh_endpoint`. The smoke-test tool has its own n0 builder (`crates/smoke-tests/src/main.rs:222`); it is a test tool and is not covered. | `crates/router/tests/no_n0_relay_preset.rs`, `crates/coordinator_iroh/tests/startup_config.rs` |
| AC-11 | Boot does not wait for the registry. It starts with the saved choice, or with an empty relay map, and resolves in the background. A relay inserted later becomes the home relay. | |
| AC-12 | The default of `registry_url` stays `None`. The substrate never contacts a registry or a relay that the operator did not configure or that a configured registry did not list. | |
| AC-13 | The registry config check fails at startup for a list entry with a bad URL, or a duplicate after URL normalisation. | `crates/core/src/curated_list/tests.rs` (the 4 `check_entries_*` tests), `crates/community_registry/src/curated_list/tests.rs` (`bad_config_or_duplicate_urls_stop_initialization`), `crates/substrate/tests/curated_list_e2e.rs` (`duplicate_coordinator_urls_stop_substrate_init`) |
| AC-14 | `coordinator_discovery_url` is removed from the config. | `crates/core/src/config/tests.rs` |
| AC-15 | A registry with an empty list asks its parent registry and returns that answer. Forwarding stops after 3 hops. | `crates/community_registry/src/curated_list/tests.rs` (`empty_list_forwards_to_parent_registry`, `hop_limit_stops_forwarding_chain`, `loop_prevention_at_hop_limit_does_not_call_parent`) |
| AC-16 | A coordinator role (Iroh or WebRTC) never resolves its own parent relay from the registry, even when `url` is not set. | |
| AC-17 | `GET /v1/info` reports the number of relay clients. The registry lookup inside it is cached, so many probes do not cause many lookups. | |

## Design

### Registry side

The list lives in the registry role config. This is a hand-written file, so the operator decides what is listed. There is no self-registration in this version.

```toml
[[roles.community_registry.coordinators]]
transport = "iroh-relay"
url = "https://relay.example.org"
info_url = "https://relay.example.org:7974"
operator = "Example Org"
```

The endpoint returns:

```json
{ "max_age_secs": 86400, "coordinators": [ { "transport": "iroh-relay", "url": "https://relay.example.org", "info_url": "https://relay.example.org:7974", "operator": "Example Org" } ] }
```

Only open relays are listed. The URL is the entry id. The operator must only list a coordinator whose `[roles.coordinator] access` is `"everyone"`.

A registry with an empty list forwards the request to its parent URL. This is new behaviour: today only registration is passed upward (`crates/community_registry/src/registry.rs:226-229,287`), and `lookup` never asks the parent. A hop counter in the request stops a loop after 3 hops.

This list is separate from `share_in_registry`. That setting makes a coordinator publish its own endpoint record so that other nodes can find it by id. It does not say "this is a relay for anyone to use".

### URL form and ranking

Normalise every URL before it is ranked and before the duplicate check: lower-case the scheme and host, drop a default port, and drop a trailing slash. `RelayUrl` adds a trailing slash, so `x` and `x/` would otherwise rank differently.

Rank key for an entry: `SHA-256( len(did) || did || len(url) || url )`, where `did` is the substrate DID, `url` is the normalised URL, and `len` is a 4-byte big-endian length. Sort the entries by this key, smallest first. Do not use Rust's default hasher: it is not stable across versions. The chooser string for coordinator ranking is the substrate's DID string (`derive_did_key`, format `did:key:h…`).

Note for Slice A4: The normalised URL has no trailing slash, but the published record uses `RelayUrl` form with a trailing slash. Slice A4 must compare a saved choice and a list entry through `normalize_url`, never as raw strings, and must parse the chosen URL into `RelayUrl` before `HomeRelay::set`.

Note for Slice A4: An empty list is cached with the normal `max_age_secs`. Slice A4 must not treat an empty list as a usable saved list, to avoid blocking retry with backoff.

### Substrate side

Resolution order for the Iroh transport:

1. `parent_coordinator.iroh.url` is set. Use it. Done.
2. A saved choice exists and its entry still answers `info_url`. Use it.
3. A saved list exists. Rank it, check entries in order, save the first good one. An expired list counts as usable when the refresh fails.
4. Ask the configured `registry_url` for the list. Save it. Then continue as in step 3.
5. Nothing worked. Degraded mode: empty relay map, warning in the log, background retry only when `registry_url` is set.

**Boot and runtime change.** Today the endpoint is built once with a fixed relay (`crates/router/src/connection_router.rs:81`), and the relay URL in the published record is read once (`crates/substrate/src/runtime/router.rs:220`). The change: the substrate boots with the saved choice, or with an empty relay map, and resolves in a background task. When a relay is chosen or changed, the task calls `Endpoint::insert_relay` and `remove_relay` (both exist in iroh 0.97) and publishes the record again.

**Home-relay handle.** The router builds the endpoint with an empty relay map and exposes a handle that inserts and removes the home relay. The record publisher reads the relay URL from a `watch` channel, and the existing republish trigger (`set_republish_trigger`) publishes again when it changes. A fixed `url` goes through the same handle: it is inserted at boot, so there is one code path. The source of the home relay is one enum, for example `HomeRelaySource::{Fixed { url }, Registry}`. The [operator-passport](../operator-passport/change.md) change adds a registration step to `Fixed` (register, then insert the relay), so the handle must allow a step before the insert.

**Ranking.** Rendezvous hashing: when the list changes, only the substrates whose choice was removed move. The sorted order is also the failover order.

**Opt-out.** There is no new flag. To run without Iroh, remove `iroh` from `communication_interfaces`. A config with no `[parent_coordinator.iroh]` section used to mean "no Iroh endpoint". After this change it means "resolve from the registry". Test configs that rely on the old meaning must be checked. The dev profile (no `--config`) gets its `localhost` relay from `Default` today. Write it into the dev profile explicitly once `url` is optional.

**Where the files go.** The cache is a small file under `config.storage.db_dir` (`coordinators.json`: list, fetch time, `max_age_secs`, chosen URL). There is no single data-directory accessor; the runtime uses `config.storage.db_dir` (`crates/core/src/config/base.rs:30`).

**Degraded mode.** The endpoint has a relay transport but no home relay. It can dial a peer through that peer's relay (the relay actor opens a connection to any URL a peer gives). Peers cannot reach it through a relay until a home relay is inserted, so it is reachable only by the direct addresses in its record. Verified by `crates/coordinator_iroh/tests/relay_map_runtime.rs`: a relay inserted later with `insert_relay` becomes the home relay within ~300 ms, `Endpoint::online()` finishes, and peers reach the endpoint through that relay.

**Why not `RelayMode::Disabled`.** In iroh 0.97 this mode builds no relay transport at all: neither listening nor dialing through relays is possible (`iroh-0.97.0/src/endpoint.rs:1721`, `socket/transports.rs:209-212`). A later `insert_relay` would only change a list with no transport behind it, and the node could not dial a peer that is behind NAT.

**Removing the n0 fallback.** Two places build the endpoint with the n0 preset today: `build_iroh_endpoint` (`crates/router/src/net_iroh.rs:99-108`, when no URL is given or the URL does not parse) and the coordinator with no parent and its relay off (`crates/coordinator_iroh/src/coordinator.rs:77`). Every runtime caller goes through `net_iroh::build_iroh_endpoint`: the substrate router, the Iroh coordinator, and the two WebRTC coordinator callers (`crates/coordinator_webrtc/src/coordinator.rs:72`, `bootstrap.rs:281`). They change to `RelayMode::Custom(RelayMap::empty())`, so a coordinator with no parent can still dial substrates through their relays.

### Rejected options

- **Random choice on every start.** The relay URL is part of the signed endpoint record. A new relay on each start means a new record and cold caches for every caller.
- **A `requires_passport` field in the list.** A passport coordinator is never found through the registry, so the field has no use.
- **n0 public relays as a fallback.** They show metadata to a third party outside the community.
- **`RelayMode::Disabled` for degraded mode, boot and the n0 replacement.** It builds no relay transport (see Design).
- **A separate opt-out flag.** Removing `iroh` from `communication_interfaces` already does this.
- **Reusing `share_in_registry` as the list.** It is a different thing (see above). It also expires after 2 hours.
- **Probing `/v1/info` on the relay's own port.** `/v1/info` has its own bind address (`info_http_bind_address`, or the relay port plus 10), which is why each entry has `info_url`.

## Open questions

- Does the list entry need `bootstrap_url` for the later WebRTC work? Not for this version.

Decided 2026-10-11:

- **Crate homes.** The ranking helper, the URL normaliser and the list wire types go in `crates/core`, because the registry, the router, the SDK and `roymctl` all depend on it, and the [service-mailbox](../service-mailbox/change.md) change reuses them for `--mailbox auto`. The list endpoint and its parent forwarding go in `crates/community_registry`, written generic over the entry type, so the mailbox list is a second instance. The resolve-and-cache task goes in `crates/substrate`, which owns the endpoint handle and the record publisher. The reads of `parent_coordinator.iroh.url` today are in `connection_router.rs:81`, `runtime/router.rs:220`, `route_handler.rs:370`, and both coordinator roles.

## Tasks

- [x] Run degraded mode once (empty relay map, then `insert_relay`) and record the result here.
- [x] Config: make `url` optional, remove `coordinator_discovery_url`, add the registry `coordinators` list and its checks (URL normalisation, duplicates). Parse `[roles.coordinator] access` strictly, in the final shape from [operator-passport](../operator-passport/change.md) (H-6), because an unknown value opens the relay to everyone today and the list relies on `access = "everyone"` being exact.
- [x] Registry: `GET /coordinators` and parent forwarding with a hop limit.
- [ ] Coordinator info endpoint: relay client count for `at_capacity`, cache the registry lookup.
- [ ] Substrate: background resolve, rank, cache, fail over, degraded mode, relay switch at run time, publish the record again.
- [x] Remove both n0 fallbacks. Check the tests that rely on them.
- [x] Coordinator roles: never auto-resolve. Test configs that rely on a missing section.
- [x] Write the dev profile relay explicitly.
- [ ] Integration tests (registry with a list, substrate with no `url`, relay that is down, registry that is down, relay change at run time). Use `common::alloc_ports`.
- [x] Update the living docs listed in the close-out.

## Deviations

- D2: an invalid `parent_coordinator.iroh.url` stops startup with an error naming the setting and value, rather than starting with an empty relay map. This changes the AC-10 text (which originally said empty relay map on invalid URL).
- D3: relay URLs are normalized to `RelayUrl` form (trailing slash appended), which reaches `/v1/info`, the signed endpoint record, and the coordinator registry registration.
- D4: publication of the substrate's own endpoint record to the community registry is deferred until a home relay is confirmed by the watch channel. Hosted service endpoints continue to be replayed.
- D5: on candidate relay confirmation failure during a switch from an active confirmed relay, the endpoint rolls back to the previous confirmed relay. Known limit: the rollback itself is not confirmed; if the old relay does not answer in the rollback's report, `set` reads `addr()` and logs a warning if it does not list the old relay.
- D6: the Iroh coordinator does not wait for `online()` at boot. A background task logs when the endpoint comes online.
- D10: `HomeRelay::set` confirmation timeout is 15 seconds (`HOME_RELAY_CONFIRM_TIMEOUT`) and QAD stays on. Measured confirm times in tests: first `set(r1)` ~3.01 s, switch `set(r2)` ~6.01 s.
- D11: the publish loop wakes on relay changes directly via `relay_rx.changed()` in the same `select!`, instead of pushing requests into the republish trigger channel.
- Confirmed home relay does not prove access: confirmation occurs when net-report probes reach the relay, before the relay connection opens and before the relay runs its access checks. Registration success, not relay confirmation, proves access (relevant for B3).
- `remove_relay` on the home relay: when all relays are removed, leaving an empty relay map, iroh 0.97 does not clear the previous home relay from `addr().relay_urls()` because it skips net-report runs when the relay map is empty; inserting a replacement relay updates the home relay immediately (`crates/coordinator_iroh/tests/relay_map_runtime.rs`).
- `[roles.coordinator] access`: list entries admit valid Iroh endpoint IDs for relay access; DID-formatted strings will not parse as endpoint IDs because relay admission checks Iroh endpoint IDs directly.
- D-A2-1: Config setting for list cache duration is named `list_max_age_secs` (default 86,400 seconds).
- D-A2-2: Forwarding between registries uses the `hops` query parameter (`0`..`3`), forwarding while `hops < 3` and passing `hops + 1`.
- D-A2-3: When a parent registry fails, is unreachable, or returns an invalid body, the registry returns `502 Bad Gateway`.
- D-A2-4: Forwarded responses are passed through as-is without local caching or body size limit.
- D-A2-5: URL normalisation lowercases scheme and host, removes default ports (80 for http, 443 for https), strips trailing slashes, and rejects userinfo, query, fragment, and non-http(s) schemes.
- D-A2-6: Both `url` and `info_url` are checked and normalised at startup.
- D-A2-7: Top-level JSON array key is derived dynamically from `ListEntry::LIST_KEY`.
- D-A2-8: The `transport` parameter on `GET /coordinators` is required and strict (`iroh-relay`).
- D-A2-9: `CoordinatorEntry` ignores unknown fields during deserialization for forward compatibility.
- D-A2-10: An empty list is served with normal `max_age_secs`.
- D-A2-11: `serde.workspace = true` is added to `crates/community_registry/Cargo.toml`; `Url::query_pairs_mut()` is used instead of `reqwest::RequestBuilder::query`.
- D-A2-12: The chooser string is fixed to the substrate's DID string (`derive_did_key`, `did:key:h…`).
- D-A2-13: The entry ID for rendezvous ranking is `url` alone.
- `list_route` is `pub(crate)` within `crates/community_registry`, scoped to the registry's own router assembly.
- Test helper `spawn_list_registry` takes a third parameter (`list_max_age_secs: Option<u64>`) to configure custom TTLs in tests.
- Note for A4 (forward timeout budget): every hop uses the 10 s timeout (`HTTP_REQUEST_TIMEOUT`). Multi-hop forward chains can exceed a client's 10 s timeout. Slice A4 must design hop-budgeted or decreasing forward timeouts for substrate discovery.

## Close-out

Living docs that are now false or incomplete when this ships:

- [system-architecture.md](../../../system-architecture.md): the config table, the "Envisioned" notes on `coordinator_discovery_url` and on a relay given by a fixed setting, the default relay paragraph (n0 preset), and the "Registry Entries at Deployment" steps.
- [system-requirements-spec.md](../../../system-requirements-spec.md): the Relay and Bootstrap sections (the note that substrates connect to a statically configured relay).
- [developer-guide.md](../../../developer-guide.md) if it lists these settings.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "home relay" and "degraded mode".

Backlog: the row "Config flags and options that nothing reads" does not mention `coordinator_discovery_url`, so it needs no change. Reduce the row "Requirements spec still describes the relay and bootstrap design as built" to what remains. The rows "Registry-listed coordinators: later improvements" and "Registry-listed coordinators for the WebRTC signalling server" are added with this change.
