---
status: draft
living-docs-touched:
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
| AC-1 | `parent_coordinator.iroh.url` is optional. When it is set, the substrate uses it and never asks the registry. | |
| AC-2 | When `url` is not set and `iroh` is in `communication_interfaces`, the substrate resolves a relay in this order: saved choice, then saved list, then the registry. | |
| AC-3 | The registry returns its configured list from `GET /coordinators?transport=iroh-relay`. Each entry has `transport`, `url`, `info_url` (where `GET /v1/info` is served, because it has its own port), and `operator`. The response says how long a client may cache it (`max_age_secs`). | |
| AC-4 | The choice is deterministic. The substrate ranks the entries by SHA-256 over a fixed byte layout (see Design) and takes the first. The same inputs always give the same result on every version of the compiler. | |
| AC-5 | The substrate keeps the list and its choice on disk. It does not ask the registry again until `max_age_secs` has passed. If a refresh fails, the old list stays in use, also after it has expired. | |
| AC-6 | Before it uses an entry, the substrate reads `info_url`. It skips an entry that cannot be reached or reports `at_capacity`, and takes the next entry in rank order. `at_capacity` is computed from the number of relay clients, not router connections. | |
| AC-7 | A saved choice is kept while it works, even if a new list changes the ranking. The substrate picks again only when the choice is removed from the list, or after 3 connection failures within 5 minutes. | |
| AC-8 | When the relay changes at run time, the endpoint switches relay without a restart. The substrate then publishes its endpoint record again with the new relay URL. | |
| AC-9 | The substrate is in degraded mode only when all three are true: there is no `url`, there is no saved choice or list, and there is no `registry_url` or the registry cannot be reached. In degraded mode the endpoint has relays disabled, the log has a clear warning, and the substrate retries the registry with backoff when `registry_url` is set. | |
| AC-10 | The endpoint is never built with the public n0 preset, with or without a relay URL, and also when a relay URL does not parse. | |
| AC-11 | Boot does not wait for the registry. It starts with the saved choice, or with relays disabled, and resolves in the background. | |
| AC-12 | The default of `registry_url` stays `None`. The substrate never contacts a registry or a relay that the operator did not configure or that a configured registry did not list. | |
| AC-13 | The registry config check fails at startup for a list entry with a bad URL, or a duplicate after URL normalisation. | |
| AC-14 | `coordinator_discovery_url` is removed from the config. | |
| AC-15 | A registry with an empty list asks its parent registry and returns that answer. Forwarding stops after 3 hops. | |
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

A registry with an empty list forwards the request to its parent URL, like `lookup` does. A hop counter in the request stops a loop after 3 hops.

This list is separate from `share_in_registry`. That setting makes a coordinator publish its own endpoint record so that other nodes can find it by id. It does not say "this is a relay for anyone to use".

### URL form and ranking

Normalise every URL before it is ranked and before the duplicate check: lower-case the scheme and host, drop a default port, and drop a trailing slash. `RelayUrl` adds a trailing slash, so `x` and `x/` would otherwise rank differently.

Rank key for an entry: `SHA-256( len(did) || did || len(url) || url )`, where `did` is the substrate DID, `url` is the normalised URL, and `len` is a 4-byte big-endian length. Sort the entries by this key, smallest first. Do not use Rust's default hasher: it is not stable across versions.

### Substrate side

Resolution order for the Iroh transport:

1. `parent_coordinator.iroh.url` is set. Use it. Done.
2. A saved choice exists and its entry still answers `info_url`. Use it.
3. A saved list exists. Rank it, check entries in order, save the first good one. An expired list counts as usable when the refresh fails.
4. Ask the configured `registry_url` for the list. Save it. Then continue as in step 3.
5. Nothing worked. Degraded mode: relays disabled, warning in the log, background retry only when `registry_url` is set.

**Boot and runtime change.** Today the endpoint is built once with a fixed relay (`crates/router/src/connection_router.rs:81`), and the relay URL in the published record is read once (`crates/substrate/src/runtime/router.rs:220`). The change: the substrate boots with the saved choice, or with relays disabled, and resolves in a background task. When a relay is chosen or changed, the task calls `Endpoint::insert_relay` and `remove_relay` (both exist in iroh 0.97) and publishes the record again.

**Ranking.** Rendezvous hashing: when the list changes, only the substrates whose choice was removed move. The sorted order is also the failover order.

**Opt-out.** There is no new flag. To run without Iroh, remove `iroh` from `communication_interfaces`. A config with no `[parent_coordinator.iroh]` section used to mean "no Iroh endpoint". After this change it means "resolve from the registry". Test configs that rely on the old meaning must be checked. The dev profile (no `--config`) gets its `localhost` relay from `Default` today. Write it into the dev profile explicitly once `url` is optional.

**Where the files go.** The cache is a small file under `config.storage.db_dir` (`coordinators.json`: list, fetch time, `max_age_secs`, chosen URL). There is no single data-directory accessor; the runtime uses `config.storage.db_dir` (`crates/core/src/config/base.rs:30`).

**Degraded mode.** With relays disabled and no address lookup, a peer can only reach the node through the direct addresses in its record. Outbound calls work. *Unverified:* run this once and confirm. The API exists (`RelayMode::Disabled`, iroh 0.97).

**Removing the n0 fallback.** Two places build the endpoint with the n0 preset today: `build_iroh_endpoint` (`crates/router/src/net_iroh.rs:99-108`, when no URL is given or the URL does not parse) and the coordinator with no parent and its relay off (`crates/coordinator_iroh/src/coordinator.rs:77`). Both change to `RelayMode::Disabled`.

### Rejected options

- **Random choice on every start.** The relay URL is part of the signed endpoint record. A new relay on each start means a new record and cold caches for every caller.
- **A `requires_passport` field in the list.** A passport coordinator is never found through the registry, so the field has no use.
- **n0 public relays as a fallback.** They show metadata to a third party outside the community.
- **A separate opt-out flag.** Removing `iroh` from `communication_interfaces` already does this.
- **Reusing `share_in_registry` as the list.** It is a different thing (see above). It also expires after 2 hours.
- **Probing `/v1/info` on the relay's own port.** `/v1/info` has its own bind address (`info_http_bind_address`, or the relay port plus 10), which is why each entry has `info_url`.

## Open questions

- Which crate owns the resolve-and-cache code: `crates/router`, `crates/substrate`, or a small new module in `crates/core`? The reads of `parent_coordinator.iroh.url` are in `connection_router.rs:81`, `runtime/router.rs:220`, `route_handler.rs:370`, and both coordinator roles.
- Does the list entry need `bootstrap_url` for the later WebRTC work? Not for this version.

## Tasks

- [ ] Run degraded mode once (relays disabled) and record the result here.
- [ ] Config: make `url` optional, remove `coordinator_discovery_url`, add the registry `coordinators` list and its checks (URL normalisation, duplicates).
- [ ] Registry: `GET /coordinators` and parent forwarding with a hop limit.
- [ ] Coordinator info endpoint: relay client count for `at_capacity`, cache the registry lookup.
- [ ] Substrate: background resolve, rank, cache, fail over, degraded mode, relay switch at run time, publish the record again.
- [ ] Remove both n0 fallbacks. Check the tests that rely on them.
- [ ] Coordinator roles: never auto-resolve. Test configs that rely on a missing section.
- [ ] Write the dev profile relay explicitly.
- [ ] Integration tests (registry with a list, substrate with no `url`, relay that is down, registry that is down, relay change at run time). Use `common::alloc_ports`.
- [ ] Update the living docs listed in the close-out.

## Deviations

## Close-out

Living docs that are now false or incomplete when this ships:

- [system-architecture.md](../../../system-architecture.md): the config table, the "Envisioned" notes on `coordinator_discovery_url` and on a relay given by a fixed setting, the default relay paragraph (n0 preset), and the "Registry Entries at Deployment" steps.
- [system-requirements-spec.md](../../../system-requirements-spec.md): the Relay and Bootstrap sections (the note that substrates connect to a statically configured relay).
- [developer-guide.md](../../../developer-guide.md) if it lists these settings.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "home relay" and "degraded mode".

Backlog: the row "Config flags and options that nothing reads" does not mention `coordinator_discovery_url`, so it needs no change. Reduce the row "Requirements spec still describes the relay and bootstrap design as built" to what remains. The rows "Registry-listed coordinators: later improvements" and "Registry-listed coordinators for the WebRTC signalling server" are added with this change.
