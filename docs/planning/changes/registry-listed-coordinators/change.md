---
status: draft
living-docs-touched:
---
# Registry-listed coordinators

*Reader: developers and operators.*

## Problem

A substrate needs a home relay (an Iroh relay) to be reachable from outside its network. Today the operator must set `parent_coordinator.iroh.url` by hand. A substrate with no such setting builds no Iroh endpoint at all. If the section exists without `url`, the value is `http://localhost:7964`, which is only useful for development.

The architecture doc already plans a better way: a substrate asks its registry for coordinators, picks one and caches it ([system-architecture.md](../../../system-architecture.md), "Registry Entries at Deployment" section). The config key `coordinator_discovery_url` was added for this, but no code reads it, and no code lists, selects or caches coordinators.

This change builds that for the Iroh relay. A community registry operator lists open coordinators. A substrate with no coordinator of its own uses one of them.

## Scope and non-goals

- In scope:
  - A curated list of open Iroh relays in the community registry config.
  - A registry HTTP endpoint that returns the list.
  - On the substrate: choose one relay from the list, cache the list and the choice on disk, and fail over.
  - Change `parent_coordinator.iroh.url` to optional, with the fallback when it is not set.
  - Remove the unused `coordinator_discovery_url` setting.
- Not in scope:
  - Coordinators that need a passport. The registry never lists them. A substrate learns about them directly from its owner and connects with an explicit `url` ([operator-passport](../operator-passport/change.md)).
  - The WebRTC signalling server. It is unchanged in this version. Fallback for it is in the [deferred backlog](../../deferred-backlog.md).
  - Coordinators that offer themselves to the registry, registry-side health checks, capacity or region hints, weighted choice, and signed lists (deferred backlog).
  - `SyneroymClient`. It has no home relay and does not use this feature.
  - The public n0 relays as a last resort. We do not use them.
  - Open mailboxes. A second curated list for them is in [service-mailbox](../service-mailbox/change.md). It shares the ranking helper written here.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| AC-1 | `parent_coordinator.iroh.url` is optional. When it is set, the substrate uses it and never asks the registry. | |
| AC-2 | When `url` is not set and `iroh` is in `communication_interfaces`, the substrate resolves a relay in this order: cached choice, then the registry list. | |
| AC-3 | The registry returns its configured list from `GET /coordinators?transport=iroh-relay`. Each entry has `transport`, `url` and `operator`. The response says how long a client may cache it (`max_age_secs`). | |
| AC-4 | The choice is deterministic. The substrate ranks the entries by `hash(substrate_did, entry_url)` and takes the first. The same inputs always give the same result. | |
| AC-5 | The substrate keeps the list and its choice on disk. It does not ask the registry again until `max_age_secs` has passed. If a refresh fails, the old list stays in use. | |
| AC-6 | Before it uses an entry, the substrate reads `GET /v1/info` on it. It skips an entry that cannot be reached or reports `at_capacity`, and takes the next entry in rank order. | |
| AC-7 | A saved choice is kept while it works, even if a new list changes the ranking. The substrate picks again only when the choice is removed from the list, or after repeated connection failures. | |
| AC-8 | When the relay URL changes, the substrate publishes its endpoint record again with the new relay URL. | |
| AC-9 | With no `url`, no usable cache and no `registry_url`, or when the registry is unreachable, the substrate starts in degraded mode. It logs a clear warning and has no home relay. If a `registry_url` is set, it retries with backoff and switches to a relay when the registry answers. | |
| AC-10 | The default of `registry_url` stays `None`. The substrate never contacts a registry or a relay that the operator did not configure or that a configured registry did not list. | |
| AC-11 | The registry config check fails at startup for a list entry with a bad URL or a duplicate URL. | |
| AC-12 | `coordinator_discovery_url` is removed from the config. | |
| AC-13 | A registry with an empty list asks its parent registry (the parent URL it already uses to pass registrations upward) and returns that answer. | |

## Design

### Registry side

The list lives in the registry role config. This is a hand-written file, so the operator decides what is listed. There is no self-registration in this version.

```toml
[[roles.community_registry.coordinators]]
transport = "iroh-relay"
url = "https://relay.example.org"
operator = "Example Org"
```

The endpoint returns:

```json
{ "max_age_secs": 86400, "coordinators": [ { "transport": "iroh-relay", "url": "https://relay.example.org", "operator": "Example Org" } ] }
```

Only open relays are listed. The URL is the entry id. There is no separate id field. The operator must only list a coordinator whose `[roles.coordinator] access` is `"everyone"`.

This list is separate from `share_in_registry`. That setting makes a coordinator publish its own endpoint record so that other nodes can find it by id. It does not say "this is a relay for anyone to use".

### Substrate side

Resolution order for the Iroh transport:

1. `parent_coordinator.iroh.url` is set. Use it. Done.
2. A saved choice exists and its entry still answers `/v1/info`. Use it.
3. A saved list exists and has not expired. Rank it, check entries in order, save the first good one.
4. Ask the configured `registry_url` for the list. Save it. Then continue as in step 3.
5. Nothing worked. Degraded mode: no home relay, warning in the log, background retry only when `registry_url` is set.

**Ranking.** Rendezvous hashing: compute `hash(substrate_did, entry_url)` for every entry and sort. It needs no stored state. When the list changes, only the substrates whose choice was removed move. The sorted order is also the failover order.

**Opt-out.** There is no new flag. To run without Iroh, remove `iroh` from `communication_interfaces`. The dev profile (no `--config`) keeps its explicit `localhost` setting.

**Where the files go.** The cache is a small file in the substrate data directory (`coordinators.json`: list, fetch time, `max_age_secs`, chosen URL). *Unverified:* the exact data directory accessor in `crates/substrate`.

**Degraded mode.** *Unverified:* that an Iroh endpoint can be built with relays turned off and still serve direct and LAN connections. Check this before writing the task for step 5.

### Rejected options

- **Random choice on every start.** The relay URL is part of the signed endpoint record. A new relay on each start means a new record and cold caches for every caller.
- **A `requires_passport` field in the list.** A passport coordinator is never found through the registry, so the field has no use.
- **n0 public relays as a fallback.** They show metadata to a third party outside the community.
- **A separate opt-out flag.** Removing `iroh` from `communication_interfaces` already does this.
- **Reusing `share_in_registry` as the list.** It is a different thing (see above). It also expires after 2 hours.

## Open questions

- Which crate owns the resolve-and-cache code: `crates/router`, `crates/substrate`, or a small new module in `crates/core`? Find where `parent_coordinator.iroh.url` is read today.
- How many failures count as "repeated" in AC-7? Suggest 3 over 5 minutes. Check [ADR-0003](../../../decisions/0003-retry-policy-ownership.md) before choosing.
- A relay is listed at `https://…`. Does `/v1/info` live at the same host and port? Today it has its own bind address (`info_http_bind_address`). The list entry may need an `info_url` field.

## Tasks

- [ ] Verify the two *unverified* items.
- [ ] Config: make `url` optional, remove `coordinator_discovery_url`, add the registry `coordinators` list and its checks.
- [ ] Registry: `GET /coordinators` and parent forwarding.
- [ ] Substrate: resolve, rank, cache, fail over, degraded mode.
- [ ] Publish the endpoint record again when the relay changes.
- [ ] Integration tests (registry with a list, substrate with no `url`, relay that is down, registry that is down). Use `common::alloc_ports`.
- [ ] Update the living docs listed in the close-out.

## Deviations

## Close-out

Living docs that are now false or incomplete when this ships:

- [system-architecture.md](../../../system-architecture.md): the config table, the "Envisioned" notes on `coordinator_discovery_url` and on a relay given by a fixed setting, and the "Registry Entries at Deployment" steps.
- [system-requirements-spec.md](../../../system-requirements-spec.md): the Relay and Bootstrap sections (the note that substrates connect to a statically configured relay).
- [developer-guide.md](../../../developer-guide.md) if it lists these settings.

Remove the backlog rows "Config flags and options that nothing reads" (the `coordinator_discovery_url` part) and "Requirements spec still describes the relay and bootstrap design as built" when this is done, or reduce them to what remains. The new backlog row "Registry-listed coordinators: later improvements" is already added.
