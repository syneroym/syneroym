---
status: done
living-docs-touched:
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/developer-guide.md
  - docs/TERMINOLOGY.md
  - docs/planning/traceability-matrix.md
  - docs/demo/demo.md
demo:
  - none: before demo stories
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
| AC-1 | `parent_coordinator.iroh.url` is optional. When it is set, the substrate uses it and never asks the registry. | `crates/core/src/config/tests.rs`, `crates/substrate/src/runtime/home_relay/tests.rs` (`url_is_a_fixed_source`), `crates/substrate/tests/home_relay_e2e.rs`, `crates/substrate/tests/home_relay_registry_e2e.rs` (`fixed_url_never_asks_the_registry`) |
| AC-2 | When `url` is not set and `iroh` is in `communication_interfaces`, the substrate resolves a relay in this order: saved choice, then saved list, then the registry. | `crates/substrate/src/runtime/home_relay/resolver/tests/boot.rs` (`saved_choice_that_answers_is_used_without_a_registry_request`, `fresh_saved_list_is_used_without_a_registry_request`, `saved_choice_off_the_saved_list_is_not_used_at_boot`, `fresh_list_with_no_working_entry_asks_the_registry`, `expired_list_is_used_when_the_registry_fails`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`picks_a_listed_relay_and_publishes_it`, `restart_uses_the_saved_choice`) |
| AC-3 | The registry returns its configured list from `GET /coordinators?transport=iroh-relay`. Each entry has `transport`, `url`, `info_url` (where `GET /v1/info` is served, because it has its own port), and `operator`. The response says how long a client may cache it (`max_age_secs`). | `crates/community_registry/src/curated_list/tests.rs` (`registry_serves_its_configured_curated_list`, `no_parent_and_empty_list_returns_empty_list_ok`), `crates/substrate/tests/curated_list_e2e.rs` (`booted_substrate_serves_curated_coordinators_list`) |
| AC-4 | The choice is deterministic. The substrate ranks the entries by SHA-256 over a fixed byte layout (see Design) and takes the first. The same inputs always give the same result on every version of the compiler. | ranking helper (`crates/core/src/curated_list/tests.rs`); substrate use: `crates/substrate/tests/home_relay_registry_e2e.rs` (`picks_a_listed_relay_and_publishes_it` compares the record with `rank`) |
| AC-5 | The substrate keeps the list and its choice on disk. It does not ask the registry again until `max_age_secs` has passed, with two exceptions: a fresh list that gives no working relay, and an empty list, both lead to a request. A refresh is at least 10 seconds after the last one. If a refresh fails, the old list stays in use, also after it has expired. | `crates/substrate/src/runtime/home_relay/resolver/tests/boot.rs` (`fresh_saved_list_is_used_without_a_registry_request`, `expired_list_is_refreshed_and_the_choice_kept_when_still_listed`, `expired_list_is_used_when_the_registry_fails`, `empty_fresh_list_asks_the_registry`, `max_age_zero_does_not_loop`), `crates/substrate/src/runtime/home_relay/resolver/tests/keep.rs` (`refresh_failure_keeps_the_relay_and_retries`), `crates/substrate/src/runtime/home_relay/cache/tests.rs`, `crates/substrate/tests/home_relay_registry_e2e.rs` (`restart_uses_the_saved_choice`, `expired_list_is_used_when_the_registry_is_down`) |
| AC-6 | Before it uses an entry, the substrate reads `info_url`. It skips an entry that cannot be reached or reports `at_capacity`, and takes the next entry in rank order. `at_capacity` is computed from the number of relay clients, not router connections. The count is the `accepts` counter minus the `disconnects` counter of `iroh-relay` (`server/metrics.rs`), which has no direct gauge. | server side: `crates/coordinator_iroh/tests/info_endpoint.rs` (`relay_count_follows_connections_and_capacity`), `crates/coordinator_iroh/tests/connection_limit.rs`, `crates/coordinator_iroh/src/info_endpoint/tests.rs`; substrate skip: `crates/substrate/src/runtime/home_relay/resolver/tests/boot.rs` (`skips_full_down_unknown_and_relay_off_entries`, `unconfirmed_relay_moves_on_to_the_next_entry`, `saved_choice_at_capacity_is_kept`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`skips_coordinators_that_are_full_or_down`, `relay_that_does_not_confirm_is_skipped`, `uses_a_real_coordinator_from_the_list`) |
| AC-7 | A saved choice is kept while it works, even if a new list changes the ranking. The substrate picks again only when the choice is removed from the list, or after 3 failed probes of its coordinator's `/v1/info` within 5 minutes. The probe runs every 60 seconds. iroh 0.97 reports no relay connection failures, so a failed probe is the only signal. A relay that is listed but refuses this node is not detected (deferred backlog). | `crates/substrate/src/runtime/home_relay/resolver/tests/keep.rs` (`two_failures_in_five_minutes_keep_the_relay`, `three_failures_in_five_minutes_pick_another_relay`, `failures_spread_over_more_than_five_minutes_keep_the_relay`, `relay_removed_from_the_list_is_replaced`, `relay_at_capacity_is_not_left_by_the_health_probe`, `failed_repick_keeps_the_current_relay`, `kept_relay_off_the_list_retries_on_the_backoff_*`, `dead_kept_relay_gets_no_cool_down`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`relay_that_leaves_the_list_is_replaced`) |
| AC-8 | When the relay changes at run time, the endpoint switches relay without a restart. The substrate then publishes its endpoint record again with the new relay URL. | `crates/substrate/tests/home_relay_e2e.rs` (`substrate_switches_home_relay_when_watch_channel_updates`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`relay_that_leaves_the_list_is_replaced`) |
| AC-9 | The substrate is in degraded mode when there is no `url` and no relay comes from the coordinator list: there is no `registry_url`, or neither the registry nor the saved list gives a usable relay. A saved list counts only when it comes from the configured `registry_url`. In degraded mode the endpoint has an empty relay map (`RelayMode::Custom(RelayMap::empty())`), the log has a clear warning (at most one every 5 minutes), and the substrate retries the registry with backoff when `registry_url` is set (5 seconds, doubling up to 10 minutes). | `crates/substrate/src/runtime/home_relay/resolver/tests/keep.rs` (`nothing_works_clears_the_map_and_retries_with_backoff`), `crates/substrate/src/runtime/home_relay/resolver/tests/logs.rs` (`degraded_mode_warns_once_per_five_minutes`), `crates/router/src/home_relay/tests.rs` (`clear_unconfirmed_*`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`degraded_until_the_registry_answers`) |
| AC-10 | The endpoint is never built with the public n0 preset, with or without a relay URL, and also when a relay URL does not parse. This covers every runtime caller of `net_iroh::build_iroh_endpoint`. The smoke-test tool has its own n0 builder (`crates/smoke-tests/src/main.rs:222`); it is a test tool and is not covered. | `crates/router/tests/no_n0_relay_preset.rs`, `crates/coordinator_iroh/tests/startup_config.rs` |
| AC-11 | Boot does not wait for the registry. It starts with the saved choice, or with an empty relay map, and resolves in the background. A relay inserted later becomes the home relay. | `crates/substrate/tests/home_relay_e2e.rs` (`a_relay_that_is_down_at_boot_is_used_once_it_comes_up`), `crates/substrate/tests/home_relay_registry_e2e.rs` (`degraded_until_the_registry_answers` boots with no ready wait) |
| AC-12 | The default of `registry_url` stays `None`. The substrate never contacts a registry or a relay that the operator did not configure or that a configured registry did not list. A relay that the registry removes from its list at run time is kept only until a listed replacement is confirmed. | `crates/substrate/src/runtime/home_relay/resolver/tests/boot.rs` (`cache_from_another_registry_is_ignored`, `saved_choice_off_the_saved_list_is_not_used_at_boot`), `crates/substrate/src/runtime/home_relay/tests.rs` (`no_url_and_no_registry_is_degraded_mode`, `webrtc_only_node_ignores_a_bad_registry_url`) |
| AC-13 | The registry config check fails at startup for a list entry with a bad URL, or a duplicate after URL normalisation. | `crates/core/src/curated_list/tests.rs` (the 4 `check_entries_*` tests), `crates/community_registry/src/curated_list/tests.rs` (`bad_config_or_duplicate_urls_stop_initialization`), `crates/substrate/tests/curated_list_e2e.rs` (`duplicate_coordinator_urls_stop_substrate_init`) |
| AC-14 | `coordinator_discovery_url` is removed from the config. | `crates/core/src/config/tests.rs` |
| AC-15 | A registry with an empty list asks its parent registry and returns that answer. Forwarding stops after 3 hops. | `crates/community_registry/src/curated_list/tests.rs` (`empty_list_forwards_to_parent_registry`, `hop_limit_stops_forwarding_chain`, `loop_prevention_at_hop_limit_does_not_call_parent`) |
| AC-16 | A coordinator role (Iroh or WebRTC) never resolves its own parent relay from the registry, even when `url` is not set. | `crates/substrate/tests/home_relay_registry_e2e.rs` (`coordinator_role_never_asks_for_the_list`) |
| AC-17 | `GET /v1/info` reports the number of relay clients. The registry lookup inside it is cached, so many probes do not cause many lookups. | `crates/coordinator_iroh/tests/info_endpoint.rs` (`relay_count_follows_connections_and_capacity`, `probes_do_not_call_the_registry`) |

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

Note for Slice A4: use an entry only when `/v1/info` answers with `status == "healthy"` and `relay.online == true`. Skip it when `status` is `at_capacity` or `Unknown` (a value this build does not know), and when `relay` is missing or `relay.online` is false: a coordinator with no relay server reports `healthy`.

Note for Slice A4: read `/v1/info` with `syneroym_core::coordinator_info::{CoordinatorInfo, INFO_PATH}`; the URL is `info_url` (normalised, no trailing slash) followed by `INFO_PATH`.

### Substrate side

Resolution order for the Iroh transport:

1. `parent_coordinator.iroh.url` is set. Use it. Done.
2. A saved choice exists and its entry still answers `info_url`. Use it.
3. A saved list exists. Rank it, check entries in order, save the first good one. An expired list counts as usable when the refresh fails.
4. Ask the configured `registry_url` for the list. Save it. Then continue as in step 3.
5. Nothing worked. Degraded mode: empty relay map, warning in the log, background retry only when `registry_url` is set.

**Boot and runtime change.** The endpoint is built with an empty relay map (`crates/router/src/connection_router.rs`). The substrate boots without waiting and resolves its relay in a background task (`crates/substrate/src/runtime/home_relay.rs`). When a relay is chosen or changed, the task calls `HomeRelay::set`, which uses `Endpoint::insert_relay` and `remove_relay` (both exist in iroh 0.97). The record publisher reads the relay from a watch channel and publishes again. The detailed behaviour of the task is in the architecture doc, section "Relay and Registry Configuration".

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

- **Crate homes.** The ranking helper, the URL normaliser and the list wire types go in `crates/core`, because the registry, the router, the SDK and `roymctl` all depend on it, and the [service-mailbox](../service-mailbox/change.md) change reuses them for `--mailbox auto`. The list endpoint and its parent forwarding go in `crates/community_registry`, written generic over the entry type, so the mailbox list is a second instance. The resolve-and-cache task goes in `crates/substrate`, which owns the endpoint handle and the record publisher. The reads of `parent_coordinator.iroh.url` are in the substrate home-relay source and in both coordinator roles. The route handler stores the value in a field that nothing reads (deferred backlog).

## Tasks

- [x] Run degraded mode once (empty relay map, then `insert_relay`) and record the result here.
- [x] Config: make `url` optional, remove `coordinator_discovery_url`, add the registry `coordinators` list and its checks (URL normalisation, duplicates). Parse `[roles.coordinator] access` strictly, in the final shape from [operator-passport](../operator-passport/change.md) (H-6), because an unknown value opens the relay to everyone today and the list relies on `access = "everyone"` being exact.
- [x] Registry: `GET /coordinators` and parent forwarding with a hop limit.
- [x] Coordinator info endpoint: relay client count for `at_capacity`, cache the registry lookup.
- [x] Substrate: background resolve, rank, cache, fail over, degraded mode, relay switch at run time, publish the record again.
- [x] Remove both n0 fallbacks. Check the tests that rely on them.
- [x] Coordinator roles: never auto-resolve. Test configs that rely on a missing section.
- [x] Write the dev profile relay explicitly.
- [x] Integration tests (registry with a list, substrate with no `url`, relay that is down, registry that is down, relay change at run time). Use `common::alloc_ports`.
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
- Note for A4 (forward timeout budget): every hop uses the 10 s timeout (`registry_http::HTTP_REQUEST_TIMEOUT`). Multi-hop forward chains can exceed a client's 10 s timeout. Slice A4 must design hop-budgeted or decreasing forward timeouts for substrate discovery.
- The registry checks `parent_registry_url` at startup with `normalize_url` and refuses to start if it is not a valid `http` or `https` URL, has a query, has a trailing dot in the host, or has consecutive slashes in the path.
- D-A3-1: Relay client cap key `[roles.coordinator.iroh] max_relay_clients: Option<u64>`, default `None` (no cap); `max_connections` continues to cap router connections only.
- D-A3-2: The relay client cap is reported via `/v1/info`, not enforced at connection admission (iroh-relay 0.97 has no client limit and access closure runs before reconnect cleanup).
- D-A3-3: Setting `max_relay_clients = 0` stops startup with an error naming the key; setting it with `enable_relay = false` logs a warning.
- D-A3-4: `/v1/info` leaves `identity` as `null` until the host substrate's node DID and controller verification are wired in Slice B2.
- D-A3-5: Registry state has its own object `registry` in `/v1/info` (`did` and `registered`), present only when the coordinator registers itself.
- D-A3-6: The coordinator refreshes its registry registration on a timer (1 hour after success, 60s after failure) rather than registering once; the `/v1/info` handler performs no network lookup.
- D-A3-7: Coordinator registry registration uses `RegistryClient::register` with standard 10s HTTP timeout and no DHT publication.
- D-A3-8: Coordinator `/v1/info` `status` evaluates relay connections against `max_relay_clients` only; router connections at `max_connections` do not yield `at_capacity`.
- D-A3-9: Relay client count is calculated from iroh-relay server metrics as `accepts.saturating_sub(disconnects)`, counting open connections rather than unique endpoints.
- D-A3-10: The coordinator's own endpoint is counted toward relay clients only when admitted by relay access (e.g. `access = "everyone"`). (Replaced by `D-B2-3` of the passport change doc: the coordinator's own endpoint is admitted under every access form.)
- D-A3-11: `/v1/info` `status` is serialized as typed `CoordinatorStatus` enum (`healthy`, `at_capacity`, `unknown` fallback); added `relay.clients`, `relay.max_clients`, and `registry`.
- D-A3-12: `/v1/info` wire types move to `crates/core::coordinator_info` so substrate resolver (Slice A4) can access them without depending on `syneroym-coordinator-iroh`.
- D-A4-1: `HomeRelaySource` has a second variant, `Registry { registry_url, cache_path }`. `from_config` takes the whole `SubstrateConfig` and returns `Fixed` when `url` is set, `Registry` when `url` is not set, `iroh` is in `communication_interfaces` and `substrate.registry_url` is set, and `None` (degraded, nothing to retry) otherwise. A node runs one source and one task.
- D-A4-2: A bad `substrate.registry_url` stops startup when it would be used (`url` not set and `iroh` in `communication_interfaces`). The error names `substrate.registry_url` and the value. A WebRTC-only node never reads it.
- D-A4-3: `coordinators.json` (in `storage.db_dir`) stores the normalised registry URL it came from. A cache from another registry is ignored. So with no `registry_url` there is no usable cache, and AC-9 is worded to match.
- D-A4-4: The health of the chosen relay comes from `/v1/info` probes every 60 seconds, because iroh 0.97 gives no signal for a failed relay connection. A probe that fails, or answers with no online relay or with status `unknown`, is one failure; 3 within 5 minutes start a new pick. Known gap: a listed relay that refuses this node still confirms and answers probes, so it is never left (deferred backlog).
- D-A4-5: To keep a saved choice, the probe must show an online relay and status `healthy` or `at_capacity`. To pick a new entry, the status must be `healthy`. After a restart the old connection may still count at the relay, so a full relay that serves this node is not left.
- D-A4-6: A fresh list where no entry works is not final. The resolver asks the registry anyway. `max_age_secs` limits refreshes only while the list gives a working relay.
- D-A4-7: An empty list is saved but never used. It leads to a registry request on every retry.
- D-A4-8: A refresh is at least 10 seconds after the last one, also when a registry sends `max_age_secs = 0`. A refresh is at most 7 days away, so a huge `max_age_secs` cannot overflow the clock.
- D-A4-9: `curated_list::fetch` sets a time limit of `10 s × (4 − hops)` for each request (`list_request_timeout`), with `RequestBuilder::timeout`, which overrides the client timeout. The substrate waits 40 s, the first forward 30 s, then 20 s and 10 s. A registry that is the first forward now waits 30 s for its parent, not 10 s.
- D-A4-10: A list request tries up to 3 times (waits of 1 s and 2 s), but only when the connection is refused and only until the registry has sent an HTTP answer once. A `502` or `503`, a timeout, or a later refused connection is not tried again. The registry of the same node binds its port after the resolver task starts, which is why the first request needs this. Failed tries are logged at `debug` only. It does not use `retry_with_backoff`, which logs a warning on every try.
- D-A4-11: `HomeRelay::clear_unconfirmed` removes a first relay that was put in the map and never confirmed. The resolver calls it after a pass that confirms nothing, so the relay map is empty in degraded mode as AC-9 says. `HomeRelay::closed` lets a background task stop when the endpoint closes.
- D-A4-12: A relay off the list gets no new connection: the saved choice is used at boot only when the saved list still lists it. A relay that is connected and working is not dropped when a refresh removes it from the list. The resolver tries to replace it on the backoff of D-A4-13, and the tries stop when the relay is listed again and its last probe passed. AC-12 is worded to match.
- D-A4-13: Retry timing. With no relay, or a kept relay that waits for a replacement: 5 seconds, doubling, at most 10 minutes, with 10 % jitter. A failed refresh while the relay works: 60 seconds, doubling, at most 1 hour. Every resolver warning goes through a `WarnLimiter`: the first one, then at most one every 5 minutes, and `debug` in between. There is one limiter each for "no home relay", "list refresh failed", "coordinator did not answer or relay not confirmed", "choosing another relay" (so the failed probes before it do not hide the decision), and "cannot save the cache". `run_fixed` uses the same limiter.
- D-A4-14: The resolver owns the `HomeRelay` handle. If a relay is confirmed that the resolver did not choose, the resolver does not call this degraded and does not clear it. It logs once at `info`. It then tries on the backoff, and replaces that relay as soon as a listed relay is confirmed.
- D-A4-15: Contract for the passport slice B3: its run-time `roymctl substrate home-relay set` must, on a node whose source is `Registry` or none, refuse or first stop the resolver, with a cancellation token that is checked between steps, never inside `HomeRelay::set`. This slice builds no way to stop the resolver. The B3 half of two backlog rows stays open. The contract is also written into the [operator-passport](../operator-passport/change.md) doc.
- D-A4-16: While a working relay is kept, an entry whose relay failed to confirm in the last hour is not tried again, because a failed switch leaves a gap of up to 15 seconds with no working relay. A relay that failed its probes gets no such cool-down.
- D-A4-17: The saved choice follows the list. Every time a list is stored, and when a saved choice is used at boot, the chosen entry is replaced by the list entry with the same normalised `url`, so a changed `info_url` or `operator` is used. Without this, a relay whose operator moved its info server could not be probed again and the node would leave it, or stay in degraded mode after a restart. When a fetched list gives the saved choice a new `info_url`, the choice is probed once more at the new address before another entry is tried. Known limit: while the saved list is still fresh the node does not fetch, so a choice whose `info_url` moved since the last fetch fails its probe at boot, and the node may move to another listed relay.
- D-A4-18: An info probe does not follow redirects (the `info_url` is written by a coordinator operator, so a redirect could aim the probe at another address of the host) and reads at most 64 KiB of the answer. The list request keeps the default client.
- D-A4-19: The resolver stops on a closed endpoint also while a list request or a probe is under way. It never drops a `HomeRelay::set`.
- Difference from the plan: the list of failed probes (`failures`) and the time of the next probe (`next_health`) live in the `Resolver`, not in `keep`. With a local list, every failed replacement try started `keep` again, and a 60-second probe timer that restarts at each try would not fire while the tries are shorter than 60 seconds. The failure list is cleared when a new pick starts.
- Difference from the plan: the test `degraded_mode_warns_once_per_five_minutes` checks one warning in the first 280 seconds and two or three in 700 seconds, not exactly two in 10 minutes. The retry delays have 10 % jitter, so the time of the second warning varies.
- Difference from the plan: the unit tests of the resolver are in `resolver/tests/{boot,keep,logs}.rs` with the fake in `resolver/tests.rs`. The `MockWriter` log capture is kept (the duplication check did not flag it). There are 12 copies of it in 5 crates, not 5 in 3 (deferred backlog).
- The test `substrate_without_home_relay_skips_registration_until_relay_configured` is replaced by `degraded_until_the_registry_answers`. After this change the harness node with `url = None` runs the `Registry` source, and a manual `home_relay().set()` would fight the resolver. The new test checks both halves through the resolver: no record before a relay is confirmed, and a record after.

## Close-out

Living docs that are now false or incomplete when this ships:

- [system-architecture.md](../../../system-architecture.md): the config table, the "Envisioned" notes on `coordinator_discovery_url` and on a relay given by a fixed setting, the default relay paragraph (n0 preset), and the "Registry Entries at Deployment" steps.
- [system-requirements-spec.md](../../../system-requirements-spec.md): the Relay and Bootstrap sections (the note that substrates connect to a statically configured relay).
- [developer-guide.md](../../../developer-guide.md) if it lists these settings.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "home relay" and "degraded mode".

Backlog: the row "Config flags and options that nothing reads" does not mention `coordinator_discovery_url`, so it needs no change. Reduce the row "Requirements spec still describes the relay and bootstrap design as built" to what remains. The rows "Registry-listed coordinators: later improvements" and "Registry-listed coordinators for the WebRTC signalling server" are added with this change.
