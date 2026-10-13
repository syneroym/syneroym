# M06D slice A4 — Substrate relay resolver: implementation plan

*Reader: the developer (or AI) who implements slice A4. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  (no "M06D", "A4", "B3", "X1", "AC-7" in code, comments or test names; ADR
  ids are fine), simple English in comments and docs, the 800-line file cap,
  the 100-line function cap, match arms of at most ~10 lines, and the module
  layout rule (`foo.rs` beside `foo/`, never `mod.rs` under `src/`).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 (rows X1, X2, X3, X7), §3 (slice A4, and B3, which adds a passport step
  to the `Fixed` source later — do not build it).
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-2, AC-5
  to AC-9, AC-11, AC-12, AC-16, "Substrate side", every "Note for Slice A4"
  line, and `## Deviations` (D2 to D11, D-A2-*, D-A3-*).
- §4 of this plan (open points). **Items marked "owner check" must be
  confirmed by the owner before you write the code they affect.**

**Branch.** A4 needs A3 (`syneroym_core::coordinator_info`), and A3 is not
in `main` yet: `main` is at `83d3ae32` (A2), and A3 is on
`feat/m06d-a3-implementation`. Do this:

- If A3 is merged into `main` when you start: branch
  `feat/m06d-a4-implementation` from `main`.
- If not: branch `feat/m06d-a4-implementation` from
  `feat/m06d-a3-implementation`, and rebase onto `main` after A3 merges.

Do not open a pull request unless the user asks. Commit messages follow the
Conventional Commits and 50/72 rules in `AGENTS.md`.

### Terms used in this plan

- **Home relay**: the one Iroh relay that the substrate's endpoint stays
  connected to. Its URL is in the substrate's signed endpoint record.
- **Coordinator list**: the curated list that a community registry serves at
  `GET /coordinators?transport=iroh-relay` (built in A2).
- **Info probe**: `GET <info_url>/v1/info` on a listed coordinator (built in
  A3). Its answer is `syneroym_core::coordinator_info::CoordinatorInfo`.
- **Chooser**: the string that ranking hashes with each entry id. For the
  coordinator list it is the substrate DID (`did:key:h…`), which is the
  `service_id` that `setup_router` already has.
- **Degraded mode**: no home relay. The endpoint has an empty relay map.

## 1. What this slice delivers

A substrate with no `parent_coordinator.iroh.url` and with
`substrate.registry_url` set finds its home relay through the coordinator
list of that registry. It runs in the background, never delays boot, keeps
its choice on disk, and fails over.

| Criterion | Where it is built | Test |
| --- | --- | --- |
| AC-1 (url set: never ask the registry) | `HomeRelaySource::from_config` | `fixed_url_never_asks_the_registry` (e2e), `home_relay/tests.rs` |
| AC-2 (order: saved choice, saved list, registry) | `Resolver::resolve_once` | resolver unit tests; e2e `restart_uses_the_saved_choice` |
| AC-4 (deterministic choice, substrate side) | `Resolver::pick_from` uses `curated_list::rank` | `picks_a_listed_relay_and_publishes_it` (e2e) compares with `rank` |
| AC-5 (cache, no ask before `max_age_secs`, old list on failure) | `cache.rs`, `resolve_once`, `keep` | unit tests; e2e `restart_uses_the_saved_choice`, `expired_list_is_used_when_the_registry_is_down` |
| AC-6 (skip unreachable / `at_capacity`, substrate side) | `ProbeVerdict`, `try_set` | unit tests; e2e `skips_coordinators_that_are_full_or_down`, `relay_that_does_not_confirm_is_skipped`, `uses_a_real_coordinator_from_the_list` |
| AC-7 (keep choice; re-pick on removal or 3 failures in 5 min) | `Resolver::keep`, `FailureWindow` | unit tests (paused time); e2e `relay_that_leaves_the_list_is_replaced` |
| AC-8 (switch at run time, publish again) | `HomeRelay::set` + existing watch channel | e2e `relay_that_leaves_the_list_is_replaced` |
| AC-9 (degraded mode, empty map, warning, backoff) | outer loop, `HomeRelay::clear_unconfirmed` | unit tests; e2e `degraded_until_the_registry_answers` |
| AC-11 (boot never waits) | task is spawned; boot not changed | e2e `degraded_until_the_registry_answers` |
| AC-12 (no unconfigured registry or relay) | `from_config`, cache bound to registry URL | unit test `cache_from_another_registry_is_ignored` |
| AC-16 (coordinator role never resolves) | no code change (structural) | e2e `coordinator_role_never_asks_for_the_list` |
| Backlog: forward timeout across hops | `curated_list::list_request_timeout` | `crates/core/src/curated_list/tests.rs` |

Not in this slice: passports (B1–B3), the mailbox list (C7), WebRTC lists, any
change to the coordinator side or to `/v1/info`.

## 2. Decisions this plan takes

Each one is recorded as a `D-A4-n` line in the change doc's `## Deviations`
when you implement it (§5).

- **D-A4-1. The source enum.** `HomeRelaySource` gets a second variant,
  `Registry { registry_url, cache_path }`. `from_config` returns:
  `Fixed` when `url` is set; `Registry` when `url` is not set, `"iroh"` is in
  `substrate.communication_interfaces`, and `substrate.registry_url` is set;
  `None` (degraded at boot, nothing to retry) otherwise. One source per boot,
  one task. The A4 half of the backlog row "`run_fixed` background task
  cancellation and single ownership" is closed by this; the B3 half stays open
  (D-A4-15).
- **D-A4-2. A bad `substrate.registry_url` stops startup** when it would be
  used: `url` is not set and `"iroh"` is in `communication_interfaces`. The
  error names `substrate.registry_url` (same rule as D2 for a bad `url`). A
  WebRTC-only node never reads it here. *Owner check (§4.6).*
- **D-A4-3. The cache is bound to its registry.** `coordinators.json` stores
  the normalised registry URL it came from. A cache from another registry is
  ignored. *Owner check (§4.2).*
- **D-A4-4. Health of the chosen relay comes from info probes.** iroh 0.97
  gives no signal for "connection to the home relay failed" (checked:
  `Endpoint` has no relay-status watcher, and `socket::Metrics` counts relay
  paths, not relay connect failures). The resolver probes the chosen entry's
  `info_url` every 60 s. A probe that fails, or answers with no online relay,
  or with `status` `unknown`, is one failure. 3 failures within 5 minutes
  start a new pick. Known gap: a listed relay that refuses this node (not
  `access = "everyone"`) is confirmed and answers probes, so it is never left
  (§4.1). *Owner check (§4.1).*
- **D-A4-5. Keeping versus picking.** To *keep* a saved choice (at boot, and
  in the 60 s health check), the probe must reach the coordinator and report
  `relay.online == true`, with `status` `healthy` **or** `at_capacity`. To
  *pick* a new entry, `status` must be `healthy`. A full relay that already
  serves this node is not left. *Owner check (§4.4).*
- **D-A4-6. A fresh list where no entry works is not final.** The resolver
  then asks the registry anyway. `max_age_secs` limits refreshes while the
  list gives a working relay. *Owner check (§4.3).*
- **D-A4-7. An empty list is stored but never used** (backlog row "Substrate
  must not treat an empty coordinator list as usable"). An empty list leads to
  a registry fetch on every retry.
- **D-A4-8. Refresh floor.** The next refresh is at least 10 s away, also when
  a registry sends `max_age_secs = 0`. This stops a tight loop.
- **D-A4-9. Decreasing forward timeouts.** A list request that carries
  `hops = h` may take `10 s × (MAX_LIST_HOPS − h + 1)`: the substrate waits
  40 s, the first forward 30 s, then 20 s, then 10 s. An inner registry gives
  up and answers `502` before the outer one stops waiting. Set per request
  with `RequestBuilder::timeout`, which overrides the client timeout
  (checked in `reqwest-0.13.5/src/async_impl/request.rs` and
  `config.rs::RequestConfig::fetch`). *Owner check (§4.7).*
- **D-A4-10. First fetch retries quickly and quietly.** The co-located
  registry binds its port in `RuntimeServices::run`, after
  `setup_connection_router` spawns the resolver, so the first fetch can be
  refused. The live fetch tries 3 times (waits 1 s, then 2 s) and logs each
  failed try at `debug` only. It does not use `retry::retry_with_backoff`,
  which logs `warn!` on every try.
- **D-A4-11. Degraded mode empties the relay map.** `HomeRelay::set` leaves an
  unconfirmed *first* relay in the map (by design, for `run_fixed`). After a
  pass that confirms nothing, the resolver calls a new
  `HomeRelay::clear_unconfirmed`, so the map is empty as AC-9 says. It does
  nothing when a relay is confirmed.
- **D-A4-12. A relay off the list gets no new connection; a working one is
  not dropped.** The saved choice is used at boot only when the saved list
  still lists it (AC-12: no *new* connection to a relay the configured
  registry does not list). At run time, when a refresh drops the chosen relay,
  or it fails 3 times, the resolver picks again. If nothing else works, the
  substrate stays on the relay it has (the `HomeRelay::set` rollback already
  keeps it): dropping a working relay would make the node unreachable for no
  gain, because its metadata was already shown to that relay. While it stays,
  it tries a new pick on the backoff of D-A4-13 (5 s, doubling, at most 10
  minutes), so at most one list fetch per try and, in steady state, one every
  10 minutes. The tries stop when the relay is listed again and its last probe
  passed. *Owner check (§4.2b).*
- **D-A4-13. Retry timing and logs.** No relay, or a kept relay waiting for
  replacement (D-A4-12): retry after 5 s, doubling, at most 10 minutes, ±10 %
  jitter. Failed refresh with a working, listed relay: retry after 60 s,
  doubling, at most 1 hour. **Every** resolver `warn!` goes through a
  `WarnLimiter`: the first one, then at most one every 5 minutes, `debug!` in
  between (the same rule as `run_fixed`). Use one limiter for "no home
  relay" (whose message includes the last fetch error, so the fetch failure
  is not logged on its own) and one for "list refresh failed".
- **D-A4-14. The resolver owns the `HomeRelay` handle.** Nothing else calls
  `set` on a node with the `Registry` source. If a relay is confirmed that the
  resolver did not choose, the resolver does not log "degraded" and does not
  clear it; it logs once at `info` and waits on the backoff.
- **D-A4-15. Contract for B3 (written into the passport doc now).** A4 does
  not build a way to stop the resolver. B3's run-time
  `roymctl substrate home-relay set` must, on a node whose source is
  `Registry` or `None`, either refuse or first stop the resolver. If B3 stops
  it, use a cancellation token checked between steps, never inside
  `HomeRelay::set` (its doc comment: dropping that future leaves the map and
  the watch channel out of step). A4 keeps the B3 half of both backlog rows
  open (§5.4).
- **D-A4-16. Confirm cool-down while a relay is kept.** A failed switch is
  not free: `HomeRelay::set` puts B in the map and takes A out, so the node
  may have no working relay for up to 15 s; then it rolls back to A, and that
  rollback is not confirmed (D5). That cost is real only while the kept relay
  still works. So `pick_from` skips an entry whose relay failed to confirm in
  the last hour (`not_confirmed: HashMap<String, Instant>` in `Resolver`,
  filled by `try_set` on `NotConfirmed`, entries older than 1 hour dropped)
  **only when** a relay is current **and** its last health probe passed
  (`Resolver.kept_relay_ok`). That is the "working relay off the list" case.
  It does not apply when there is no relay, or when the kept relay failed its
  probes (it is probably dead, so a failed switch costs nothing, and waiting
  1 hour would stretch the outage). *Owner check (§4.2b).*

## 3. Step-by-step changes

Do the steps in order. Each step names every file and every call site.

### Step 1 — forward timeout per hop (`crates/core`)

File: `crates/core/src/curated_list.rs`.

Add, after `MAX_LIST_HOPS`:

```rust
/// The total time a list request that has made `hops` forwards may take.
/// Each forward gets 10 s less than the request it serves, so an inner
/// registry gives up and answers `502` before the outer one stops waiting:
/// a client (0 hops) waits 40 s, the first forward 30 s, then 20 s, then 10 s.
#[must_use]
pub fn list_request_timeout(hops: u8) -> Duration {
    let steps = u32::from(MAX_LIST_HOPS.saturating_sub(hops)) + 1;
    registry_http::HTTP_REQUEST_TIMEOUT * steps
}
```

Imports: `use std::time::Duration;` and `use crate::registry_http;`.

In `fetch`, change the send line to:

```rust
let list = client
    .get(url)
    .timeout(list_request_timeout(hops))
    .send()
    .await?
    .error_for_status()?
    .json::<CuratedList<E>>()
    .await?;
```

Add to the `fetch` doc comment: "The request time limit is
`list_request_timeout(hops)`; it overrides the client's own timeout."

Call sites of `fetch` (no signature change, so no edit needed, but check
they still read right):

- `crates/community_registry/src/curated_list.rs` `serve_list` (passes
  `hop.hops + 1`, so a registry that received `h` forwards with
  `list_request_timeout(h + 1)`). The client built in `router()` keeps
  `HTTP_REQUEST_TIMEOUT`; it now only applies to requests without their own
  timeout. No change.
- `crates/substrate/tests/curated_list_e2e.rs` (passes 0). No change.
- The new substrate resolver (Step 5).

Test, in `crates/core/src/curated_list/tests.rs`:

```rust
#[test]
fn list_request_timeout_shrinks_with_each_hop() {
    let secs = |h| curated_list::list_request_timeout(h).as_secs();
    assert_eq!([secs(0), secs(1), secs(2), secs(3), secs(4)], [40, 30, 20, 10, 10]);
}
```

(Match the file's existing import style for `curated_list`.)

### Step 2 — two small `HomeRelay` methods (`crates/router`)

File: `crates/router/src/home_relay.rs`. Add to `impl HomeRelay`:

```rust
/// Waits until the endpoint is closed. A background task uses it to stop.
pub async fn closed(&self) {
    self.inner.endpoint.closed().await;
}

/// Removes a relay that a `set` put in the map but iroh never confirmed, so
/// the relay map is empty again. Does nothing when a relay is confirmed.
pub async fn clear_unconfirmed(&self) {
    let mut in_map = self.inner.in_map.lock().await;
    if self.inner.confirmed.borrow().is_some() {
        return;
    }
    if let Some(url) = in_map.take() {
        self.inner.endpoint.remove_relay(&url).await;
    }
}
```

Take the `in_map` lock first, as `set` does, so `clear_unconfirmed` never runs
in the middle of a `set`.

Tests, in `crates/router/src/home_relay/tests.rs` (reuse `dead_relay` and
`bind_empty`):

- `clear_unconfirmed_empties_the_map_after_a_failed_first_set`: `set(dead)`
  fails, `clear_unconfirmed()`, then `in_map()` is `None` and
  `subscribe().borrow()` is `None`.
- `clear_unconfirmed_on_a_new_handle_does_nothing`: `in_map()` stays `None`.
- `closed_returns_after_the_endpoint_closes`: spawn `handle.closed()`, call
  `ep.close().await`, the task ends within 5 s
  (`time::timeout(Duration::from_secs(5), task)`).

The "a confirmed relay stays" case needs a live relay. It is covered by the
substrate e2e tests (Step 8); this crate's unit tests have no relay server.

### Step 3 — dependencies (`crates/substrate/Cargo.toml`)

Under `[dependencies]` add `reqwest.workspace = true` and
`serde.workspace = true` (the workspace `serde` has `derive`). Remove
`reqwest.workspace = true` from `[dev-dependencies]`: normal dependencies are
visible to tests. Run `cargo shear` afterwards.

### Step 4 — the source enum (`crates/substrate/src/runtime/home_relay.rs`)

New module tree (module layout rule):

```text
crates/substrate/src/runtime/home_relay.rs          (exists: source, task start, run_fixed)
crates/substrate/src/runtime/home_relay/tests.rs    (exists: from_config tests)
crates/substrate/src/runtime/home_relay/cache.rs    (new: coordinators.json)
crates/substrate/src/runtime/home_relay/cache/tests.rs
crates/substrate/src/runtime/home_relay/io.rs       (new: ResolverIo trait + LiveIo)
crates/substrate/src/runtime/home_relay/resolver.rs (new: the resolve loop)
crates/substrate/src/runtime/home_relay/resolver/tests.rs
```

Changes in `home_relay.rs`:

```rust
mod cache;
mod io;
mod resolver;
#[cfg(test)]
mod tests;

/// The file under `storage.db_dir` that keeps the coordinator list and the
/// choice across restarts.
const COORDINATOR_CACHE_FILE: &str = "coordinators.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed { url: RelayUrl },
    /// `url` is not set and `substrate.registry_url` is: pick a relay from
    /// that registry's coordinator list.
    Registry {
        /// Normalised with `curated_list::normalize_url`.
        registry_url: String,
        cache_path: PathBuf,
    },
}

impl HomeRelaySource {
    /// The source for this config. `Ok(None)` means degraded mode with
    /// nothing to retry: no `url`, and no `substrate.registry_url` (or no
    /// Iroh endpoint at all, when the caller never uses the source).
    ///
    /// # Errors
    /// When `url` is set and not a valid relay URL, or when the registry
    /// source would be used and `substrate.registry_url` is not a valid
    /// registry URL. Startup stops.
    pub(super) fn from_config(config: &SubstrateConfig) -> anyhow::Result<Option<Self>> {
        // Unchanged from A1: a bad `url` stops startup on every node.
        if let Some(url) = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)? {
            return Ok(Some(Self::Fixed { url }));
        }
        // No Iroh endpoint means no home relay, so `registry_url` is not
        // read here (a WebRTC-only node keeps working with any value).
        let uses_iroh = config.substrate.communication_interfaces.iter().any(|c| c == "iroh");
        let Some(raw) = config.substrate.registry_url.as_deref().filter(|_| uses_iroh) else {
            return Ok(None);
        };
        let registry_url = curated_list::normalize_url(raw)
            .with_context(|| format!("substrate.registry_url = {raw:?}"))?;
        let cache_path = config.storage.db_dir.join(COORDINATOR_CACHE_FILE);
        Ok(Some(Self::Registry { registry_url, cache_path }))
    }
}
```

`connect_home_relay` gets the chooser:

```rust
pub(super) fn connect_home_relay(
    home_relay: HomeRelay,
    source: Option<HomeRelaySource>,
    chooser: &str,
)
```

- `None`: keep the one warning, new text: "`parent_coordinator.iroh.url` and
  `substrate.registry_url` are not set: the Iroh endpoint has no home relay
  (degraded mode). … Set one of them and restart, or remove \"iroh\" from
  substrate.communication_interfaces." Then return.
- `Fixed`: as today (`tokio::spawn(run_fixed(...))`).
- `Registry`: `tokio::spawn(resolver::run(home_relay, registry_url,
  cache_path, chooser.to_owned()))`.

`run_source` disappears (the match moves into `connect_home_relay`; each arm
is one line).

Extract the "warn the first time, then at most every 5 minutes" logic of
`run_fixed` into a small helper that both tasks use:

```rust
/// Warns the first time, then at most once every `WARN_INTERVAL`; logs at
/// debug level in between.
#[derive(Debug, Default)]
pub(super) struct WarnLimiter {
    /// `tokio::time::Instant`, so paused-time tests drive it. Outside
    /// paused tests it reads the same clock as `std::time::Instant`.
    last: Option<tokio::time::Instant>,
}

impl WarnLimiter {
    /// True when this message should be a warning.
    pub(super) fn should_warn(&mut self) -> bool { … }
}
```

Rename `FIXED_WARN_INTERVAL` to `WARN_INTERVAL` (300 s). `run_fixed`
behaviour does not change.

Call site to update: `crates/substrate/src/runtime/router.rs`.

- Line 127: `let relay_source = HomeRelaySource::from_config(config)?;`
- Lines 171–173:

  ```rust
  if let Some(handle) = router.home_relay() {
      home_relay::connect_home_relay(handle, relay_source, service_id);
  }
  ```

  `service_id` is the substrate DID (`setup_identity_and_storage` returns
  `substrate_identity_state.did`, made with `derive_did_key`), which is the
  chooser form that `rank_key`'s doc comment requires.

Tests, `crates/substrate/src/runtime/home_relay/tests.rs` (rewrite the three
tests for the new signature, add four, plus the limiter test):

- `no_url_and_no_registry_is_degraded_mode` → `Ok(None)`.
- `bad_url_stops_startup` (unchanged meaning).
- `url_is_a_fixed_source` (unchanged meaning). Also set `registry_url` here,
  to show `url` wins (AC-1).
- `no_url_with_registry_is_a_registry_source`: `registry_url =
  "HTTP://Reg.Example:80/"`, `db_dir = "/x/db"` → `Registry { registry_url:
  "http://reg.example", cache_path: "/x/db/coordinators.json" }`.
- `bad_registry_url_stops_startup`: `registry_url = "reg.example"` → error
  contains `substrate.registry_url` and the value.
- `webrtc_only_node_ignores_a_bad_registry_url`:
  `communication_interfaces = ["webrtc"]`, `url = None`,
  `registry_url = "reg.example"` → `Ok(None)`.
- Unit-test `WarnLimiter` with paused time: warns, then not for 299 s, then
  warns at 300 s.

Build the configs from `SubstrateConfig::default()` and set the fields.

### Step 5 — the cache file (`home_relay/cache.rs`)

```rust
//! `coordinators.json`: the last coordinator list from the registry and the
//! relay chosen from it. It lets a restart use the old choice without asking
//! the registry, and use an old list when the registry is down.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CoordinatorCache {
    /// The registry the list came from, normalised. A cache from another
    /// registry is not used.
    pub registry_url: String,
    /// Unix seconds when the list was fetched.
    pub fetched_at_secs: u64,
    /// The list as the registry sent it (entries not normalised).
    pub list: CuratedList<CoordinatorEntry>,
    /// The chosen entry, normalised (it comes out of `curated_list::rank`).
    /// `None` until an entry has worked.
    pub chosen: Option<CoordinatorEntry>,
}

impl CoordinatorCache {
    pub(super) fn expires_at_secs(&self) -> u64 {
        self.fetched_at_secs.saturating_add(self.list.max_age_secs)
    }

    pub(super) fn is_fresh(&self, now_secs: u64) -> bool {
        now_secs < self.expires_at_secs()
    }

    /// Whether the list has an entry whose normalised `url` is `url`.
    pub(super) fn lists(&self, url: &str) -> bool {
        self.list.entries.iter().any(|e| curated_list::normalize_url(&e.url).is_ok_and(|u| u == url))
    }
}

/// Reads the cache. A missing file is `None` with no log. A file that does
/// not parse is `None` with a warning (it is replaced at the next save).
pub(super) fn load(path: &Path) -> Option<CoordinatorCache>

/// Writes the cache: to `<path>.tmp` first, then renames it over `path`, so a
/// crash never leaves half a file. Creates the parent directory.
///
/// # Errors
/// When the directory or the file cannot be written.
pub(super) fn save(path: &Path, cache: &CoordinatorCache) -> anyhow::Result<()>
```

The on-disk form (from serde; shown for the docs):

```json
{
  "registry_url": "http://127.0.0.1:7961",
  "fetched_at_secs": 1792000000,
  "list": { "max_age_secs": 86400, "coordinators": [ { "transport": "iroh-relay", "url": "…", "info_url": "…", "operator": "…" } ] },
  "chosen": { "transport": "iroh-relay", "url": "…", "info_url": "…", "operator": "…" }
}
```

Use `std::fs` (blocking is fine: the file is small and written rarely; if you
prefer, `tokio::fs`). Tests in `cache/tests.rs` (with `tempfile::tempdir()`):

- `save_then_load_gives_the_same_cache`.
- `missing_file_is_none`.
- `corrupt_file_is_none` (write `"{"`).
- `save_leaves_no_temp_file`.
- `is_fresh_ends_at_max_age` (`fetched_at 100`, `max_age 10`: fresh at 109,
  not at 110).
- `lists_compares_normalised_urls` (entry `"HTTP://R.Example:80/"`,
  `lists("http://r.example")` is true).

### Step 6 — the I/O seam (`home_relay/io.rs`)

The resolver logic is time-heavy (60 s probes, 5-minute windows, backoff). To
test it with `#[tokio::test(start_paused = true)]` and no network, all its I/O
goes through one trait. Use native `async` in the trait with explicit `Send`
futures (Rust 2024; no `async-trait` crate):

```rust
/// Everything the resolver does outside itself.
pub(super) trait ResolverIo: Send + Sync + 'static {
    /// `GET <registry_url>/coordinators?transport=iroh-relay&hops=0`.
    fn fetch_list(
        &self,
        registry_url: &str,
    ) -> impl Future<Output = anyhow::Result<CuratedList<CoordinatorEntry>>> + Send;
    /// `GET <info_url>/v1/info`. `info_url` is normalised (no trailing slash).
    fn probe(&self, info_url: &str) -> impl Future<Output = anyhow::Result<CoordinatorInfo>> + Send;
    /// `HomeRelay::set`.
    fn set_relay(&self, url: RelayUrl) -> impl Future<Output = Result<(), HomeRelayError>> + Send;
    /// `HomeRelay::current_url`.
    fn current_relay(&self) -> Option<RelayUrl>;
    /// `HomeRelay::clear_unconfirmed`.
    fn clear_unconfirmed(&self) -> impl Future<Output = ()> + Send;
    /// `HomeRelay::closed`.
    fn closed(&self) -> impl Future<Output = ()> + Send;
    /// Unix seconds now. A trait method so tests can move the clock.
    fn now_secs(&self) -> u64;
}
```

The live version:

```rust
/// How long an info probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Waits between fetch tries. The first fetch can race the co-located
/// registry's bind.
const FETCH_TRY_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

pub(super) struct LiveIo {
    client: reqwest::Client,
    home_relay: HomeRelay,
}

impl LiveIo {
    /// # Errors
    /// When the HTTP client cannot be built.
    pub(super) fn new(home_relay: HomeRelay) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(registry_http::HTTP_REQUEST_TIMEOUT)
            .build()
            .context("building the coordinator-list HTTP client")?;
        Ok(Self { client, home_relay })
    }
}
```

`ResolverIo for LiveIo`:

- `fetch_list`: up to 3 tries of
  `curated_list::fetch::<CoordinatorEntry>(&self.client, registry_url,
  &CoordinatorFilter { transport: CoordinatorTransport::IrohRelay }, 0)`,
  sleeping `FETCH_TRY_WAITS[i]` between them. Log a failed try at `debug!`
  only; return the last error. Do **not** use `retry::retry_with_backoff`: it
  logs `warn!` on every try (`crates/core/src/retry.rs:42`), which would break
  the warning limit of D-A4-13.
- `probe`: `self.client.get(format!("{info_url}{INFO_PATH}")).timeout(PROBE_TIMEOUT).send().await?.error_for_status()?.json::<CoordinatorInfo>().await?`.
- the other five forward to `HomeRelay` / `SystemTime::now()`.

### Step 7 — the resolver (`home_relay/resolver.rs`)

Constants (production values; unit tests use paused time, so no test knobs):

```rust
const HEALTH_INTERVAL: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
const RETRY_INITIAL: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(600);
const REFRESH_RETRY_INITIAL: Duration = Duration::from_secs(60);
const REFRESH_RETRY_MAX: Duration = Duration::from_secs(3_600);
const MIN_REFRESH: Duration = Duration::from_secs(10);
const CONFIRM_COOLDOWN: Duration = Duration::from_secs(3_600);
```

Use `tokio::time::Instant` for every in-memory time (so paused time drives
it) and `io.now_secs()` for cache times.

Small types:

```rust
/// The endpoint closed; the task ends.
struct Closed;

/// What one info probe says about an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeVerdict {
    /// `healthy` with an online relay: may be picked.
    Open,
    /// `at_capacity` with an online relay: may be kept, not picked.
    Full,
    /// No answer, no online relay, or a status this build does not know.
    Down,
}

impl ProbeVerdict {
    fn of(answer: anyhow::Result<CoordinatorInfo>) -> Self {
        let Ok(info) = answer else { return Self::Down };
        if !info.relay.is_some_and(|r| r.online) {
            return Self::Down;
        }
        match info.status {
            CoordinatorStatus::Healthy => Self::Open,
            CoordinatorStatus::AtCapacity => Self::Full,
            CoordinatorStatus::Unknown => Self::Down,
        }
    }
    fn can_keep(self) -> bool { matches!(self, Self::Open | Self::Full) }
}

/// Failures of the chosen relay; `record` says when there are enough
/// within the window to pick again.
struct FailureWindow { times: VecDeque<Instant> }
impl FailureWindow {
    /// Adds a failure at `now`, forgets those older than `FAILURE_WINDOW`,
    /// and returns true when `FAILURE_LIMIT` are left.
    fn record(&mut self, now: Instant) -> bool
}

/// Doubling delay with ±10 % jitter (`retry::calculate_jittered_backoff`).
struct Backoff { initial: Duration, max: Duration, next: Duration }
impl Backoff {
    fn new(initial: Duration, max: Duration) -> Self
    fn next(&mut self) -> Duration   // returns jittered `next`, then doubles it, capped at `max`
    fn reset(&mut self)
}
```

For the jitter, convert with `u64::try_from(d.as_millis()).unwrap_or(u64::MAX)`
(no `as` casts).

The state:

```rust
struct Resolver<I> {
    io: I,
    chooser: String,
    registry_url: String,
    cache_path: PathBuf,
    cache: Option<CoordinatorCache>,
    /// Backoff for "no relay" and for "a kept relay waits for a
    /// replacement" (D-A4-12, D-A4-13). Reset when a pick succeeds.
    retry: Backoff,
    /// When a kept relay that is off the list or failing gets its next
    /// re-pick try. `None` when no try is due.
    repick_at: Option<Instant>,
    /// The last fetch error, shown in the "no home relay" warning instead of
    /// its own log line.
    last_fetch_error: Option<String>,
    /// Entries whose relay failed to confirm, and when (D-A4-16).
    not_confirmed: HashMap<String, Instant>,
    /// The last health probe of the current relay passed. Set `true` by a
    /// successful `try_set` (the entry was just probed) and by every health
    /// tick in `keep`; read by `pick_from` (D-A4-16) and by the repick timer
    /// in `keep`. Kept on `Resolver`, not local to `keep`, because both
    /// functions need it and it must survive a `keep` → re-pick → `keep` loop.
    /// Starts `false` (no relay yet).
    kept_relay_ok: bool,
    warn_degraded: WarnLimiter,
    warn_refresh: WarnLimiter,
    /// "Coordinator did not answer" and "relay not confirmed".
    warn_relay: WarnLimiter,
}
```

Every `warn!` below goes through one of the three limiters (`if
limiter.should_warn() { warn!(…) } else { debug!(…) }`); a small macro or
helper fn keeps this to one line per call.

Entry point, called by `connect_home_relay`:

```rust
pub(super) async fn run(home_relay: HomeRelay, registry_url: String, cache_path: PathBuf, chooser: String) {
    let io = match LiveIo::new(home_relay) {
        Ok(io) => io,
        Err(e) => {
            warn!(error = %e, "cannot resolve a home relay (degraded mode)");
            return;
        }
    };
    Resolver::new(io, registry_url, cache_path, chooser).run().await;
}
```

`Resolver::new` loads the cache and drops it when
`cache.registry_url != registry_url` (log at `info`: "ignoring
coordinators.json from another registry").

The outer loop (pseudo-code):

```text
async fn run(mut self):
    skip: Option<String> = None
    loop:
        picked = match self.resolve_once(skip.take()).await { Err(Closed) => return, Ok(p) => p }
        if picked:
            self.retry.reset(); self.repick_at = None
        let has_relay = self.io.current_relay().is_some()
        if has_relay and self.chosen().is_some():
            if !picked:
                // D-A4-12: stay on the relay we have; try again on the backoff.
                self.repick_at = Some(Instant::now() + self.retry.next())
                info!("no other listed relay works; keeping the current home relay for now")
            match self.keep().await:
                Err(Closed) => return
                Ok(url) => skip = Some(url)        // pick again, not this one
            continue
        if has_relay:
            // D-A4-14: a relay the resolver did not choose. Not degraded.
            info! once ("home relay was set outside the resolver")
        else:
            // No home relay: degraded mode (AC-9).
            self.io.clear_unconfirmed().await
            log_degraded(...)                     // through warn_degraded
        select:
            sleep(self.retry.next()) => {}
            io.closed() => return
```

`log_degraded` text: "no home relay: the coordinator list of
{registry_url} gave no usable relay (degraded mode). Peers cannot reach this
node through a relay, and its endpoint record is not published. Last list
error: {last_fetch_error or "none"}. Trying again in {delay:?}."

One resolve pass (AC-2 order):

```text
async fn resolve_once(&mut self, skip: Option<String>) -> Result<bool, Closed>:
    tried: HashSet<String> = skip into set        // normalised relay URLs
    now = io.now_secs()

    // 1. The saved choice, while the saved list still lists it and its
    //    coordinator answers. A relay off the list gets no new connection
    //    (AC-12, D-A4-12).
    if let Some(entry) = self.chosen()
            .filter(|e| !tried.contains(&e.url) && self.cache.lists(&e.url))
            .cloned():
        tried.insert(entry.url)
        if ProbeVerdict::of(io.probe(&entry.info_url).await).can_keep()
           and self.try_set(&entry).await?:
            return Ok(true)

    // 2. A fresh, non-empty saved list (no registry request).
    if let Some(list) = self.cache.filter(|c| c.is_fresh(now) && !c.list.entries.is_empty()).map(|c| c.list.clone()):
        if self.pick_from(list.entries, &mut tried).await?: return Ok(true)

    // 3. The registry. 4. On failure, the saved list at any age.
    match io.fetch_list(&registry_url).await:
        Ok(list) =>
            self.last_fetch_error = None
            self.store_list(list.clone(), io.now_secs())
            if self.pick_from(list.entries, &mut tried).await?: return Ok(true)
        Err(e) =>
            self.last_fetch_error = Some(format!("{e:#}"))   // logged by log_degraded, not here
            if let Some(list) = self.cache non-empty list (any age):
                if self.pick_from(list.entries, &mut tried).await?: return Ok(true)
    Ok(false)
```

Picking from a list:

```text
async fn pick_from(&mut self, entries, tried) -> Result<bool, Closed>:
    for entry in curated_list::rank(&self.chooser, entries):     // normalised, deduped, ranked
        if !tried.insert(entry.url.clone()): continue
        // D-A4-16: while a *working* relay is kept, do not repeat a failed
        // switch soon. A dead kept relay gets no cool-down.
        if io.current_relay().is_some() and self.kept_relay_ok
           and self.recently_not_confirmed(&entry.url): continue
        match ProbeVerdict::of(io.probe(&entry.info_url).await):
            Open => {}
            Full => { info!(relay_url = entry.url, "skipping a full coordinator"); continue }
            Down => { info!(relay_url = entry.url, "skipping a coordinator that does not answer"); continue }
        if self.try_set(&entry).await?:
            return Ok(true)
    Ok(false)

async fn try_set(&mut self, entry) -> Result<bool, Closed>:
    // A normalised URL is http(s) with a host, so this parse only fails on
    // a value `normalize_url` lets through and `RelayUrl` does not.
    let Ok(url) = net_iroh::parse_relay_url("coordinator list entry", &entry.url) else { debug!; return Ok(false) }
    match io.set_relay(url).await:
        Ok(()) =>
            info!(relay_url = entry.url, operator = entry.operator, "home relay chosen from the coordinator list")
            self.save_choice(entry.clone())
            self.kept_relay_ok = true
            Ok(true)
        Err(HomeRelayError::NotConfirmed { .. }) =>
            self.not_confirmed.insert(entry.url.clone(), Instant::now())
            warn via warn_relay; Ok(false)
        Err(HomeRelayError::EndpointClosed) => Err(Closed)
```

`set_relay` is never inside a `select!`: `HomeRelay::set` must run to the end
(its doc comment, "Cancellation").

Helpers:

- `fn chosen(&self) -> Option<&CoordinatorEntry>`.
- `fn recently_not_confirmed(&mut self, url) -> bool`: drop entries older
  than `CONFIRM_COOLDOWN` (1 hour, a new constant), then `contains_key(url)`.
- `fn store_list(&mut self, list, now_secs)`: replace or create the cache with
  `registry_url`, `fetched_at_secs = now_secs`, the list, and the old
  `chosen` (kept as is: `keep` still needs its `info_url` for health probes
  while the endpoint uses it; `resolve_once` step 1 skips it when it is not
  listed). Then `persist`.
- `fn save_choice(&mut self, entry)`: set `chosen`, then `persist`. A choice
  without a cache cannot happen (the entry came from a list), so no new cache
  here; `debug_assert!` is not needed — just do nothing when the cache is
  `None`.
- `fn persist(&self)`: `cache::save`; on error `warn!` and go on (AC-5 is
  best effort; the in-memory state still works).
- `fn refresh_delay(&self, now_secs) -> Duration`:
  `max(expires_at_secs - now_secs (saturating), 0) as Duration`, then
  `.max(MIN_REFRESH)`.

Watching the chosen relay (AC-7, D-A4-12):

```text
/// Returns the normalised URL of the chosen relay when a new pick is needed.
async fn keep(&mut self) -> Result<String, Closed>:
    chosen = self.chosen().cloned()          // caller checked it is Some
    failures = FailureWindow::default()
    // `self.kept_relay_ok` carries over from before: after a re-pick that
    // failed because of probe failures it stays `false` until a probe passes.
    refresh_retry = Backoff::new(REFRESH_RETRY_INITIAL, REFRESH_RETRY_MAX)
    health = time::interval_at(Instant::now() + HEALTH_INTERVAL, HEALTH_INTERVAL)
    health.set_missed_tick_behavior(MissedTickBehavior::Delay)
    refresh_at = Instant::now() + self.refresh_delay(io.now_secs())
    loop:
        select:
            () = io.closed() => return Err(Closed)
            _ = health.tick() =>
                self.kept_relay_ok = ProbeVerdict::of(io.probe(&chosen.info_url).await).can_keep()
                if !self.kept_relay_ok:
                    warn via warn_relay (relay_url = chosen.url, "home relay coordinator did not answer")
                    if failures.record(Instant::now()):
                        warn via warn_relay ("3 failures within 5 minutes; choosing another relay")
                        return Ok(chosen.url)
            () = time::sleep_until(refresh_at) =>
                match io.fetch_list(&registry_url).await:
                    Ok(list) =>
                        refresh_retry.reset()
                        self.store_list(list, io.now_secs())
                        refresh_at = Instant::now() + self.refresh_delay(io.now_secs())
                        // When a try is already scheduled, let the repick timer
                        // decide; returning here would skip the backoff when
                        // `max_age_secs` is short.
                        if !self.cache.lists(&chosen.url) and self.repick_at.is_none():
                            info!(relay_url = chosen.url, "home relay left the coordinator list; choosing again")
                            return Ok(chosen.url)
                    Err(e) =>
                        warn via warn_refresh (error, "cannot refresh the coordinator list; keeping the saved list")
                        refresh_at = Instant::now() + refresh_retry.next()
            // Precondition guard keeps the branch off when no try is due; use
            // `repick_at.unwrap_or(refresh_at)` (no `unwrap`, clippy denies it).
            () = time::sleep_until(self.repick_at.unwrap_or(refresh_at)), if self.repick_at.is_some() =>
                if self.kept_relay_ok and self.cache.lists(&chosen.url):
                    self.repick_at = None          // the relay is fine again: stop trying
                    self.retry.reset()
                else:
                    return Ok(chosen.url)          // the outer loop sets the next repick_at
```

Fetch rate while a relay is kept for replacement: each
re-pick try does at most one list fetch (`resolve_once` step 3; step 2 does
none). Tries follow `retry`: 5 s, 10 s, 20 s, … capped at 10 minutes. So in a
long outage the node fetches the list at most once every 10 minutes, plus one
refresh per `max_age_secs` (at least 10 s apart, D-A4-8). The refresh never
starts a re-pick of its own while a try is scheduled, so a short
`max_age_secs` does not bypass the backoff. A failure-triggered re-pick needs
3 failed probes first (at least 2 minutes after the window was last cleared).

A new list that ranks another entry first does **not** start a new pick
(AC-7). Put each arm body in a small method if a match arm grows past ~10
lines (`on_health_tick`, `on_refresh`, `on_repick_due`), returning
`Option<String>` = "pick again, skipping this URL".

Borrowing note: `tokio::select!` drops all branch futures before it runs the
chosen handler, so a handler may use `&mut self` while a branch future
borrowed `&self.io`. If the borrow checker still complains, clone what the
branch needs first.

### Step 8 — tests for the resolver

#### 8a. Unit tests, `home_relay/resolver/tests.rs`

A fake `ResolverIo`:

```rust
#[derive(Clone, Default)]
struct FakeIo(Arc<FakeState>);

#[derive(Default)]
struct FakeState {
    /// The next answer of `fetch_list`; `None` = error.
    list: Mutex<Option<CuratedList<CoordinatorEntry>>>,
    fetches: AtomicUsize,
    /// `info_url` → answer; a missing key = probe error.
    infos: Mutex<HashMap<String, CoordinatorInfo>>,
    /// Relay URLs (normalised) that `set_relay` confirms.
    confirms: Mutex<HashSet<String>>,
    /// Every `set_relay` call, normalised.
    sets: Mutex<Vec<String>>,
    current: Mutex<Option<RelayUrl>>,
    clears: AtomicUsize,
    closed: tokio_util::sync::CancellationToken,  // `tokio-util` is already a dependency
    start_secs: u64,   // `now_secs` = start_secs + tokio elapsed
}
```

`set_relay` mimics `HomeRelay`: confirmed URL → set `current`, `Ok`;
otherwise `NotConfirmed` and `current` unchanged (rollback). Helpers:
`entry(n) -> CoordinatorEntry` (`url = http://relay-n.test`, `info_url =
http://info-n.test`), `healthy()`, `full()`, `ranked(chooser, &[entries])`.
Run each test with `#[tokio::test(start_paused = true)]`, spawn
`Resolver::run`, and move time with `time::sleep` (paused time jumps
forward). End each test with `closed.cancel()` and check the task ends.

Tests (names are the behaviour; no criterion ids):

1. `saved_choice_that_answers_is_used_without_a_registry_request` — cache with
   fresh list and chosen B; `fetches == 0`; `sets == [B]`.
2. `saved_choice_at_capacity_is_kept` — chosen B reports `at_capacity`.
3. `fresh_saved_list_is_used_without_a_registry_request` — no chosen.
4. `expired_list_is_refreshed_and_the_choice_kept_when_still_listed` — new
   list ranks C before B; `sets == [B]` only.
5. `expired_list_is_used_when_the_registry_fails`.
6. `empty_fresh_list_asks_the_registry`.
7. `fresh_list_with_no_working_entry_asks_the_registry` (D-A4-6).
8. `skips_full_down_unknown_and_relay_off_entries` — four bad entries plus one
   good one in any rank order; only the good one is set.
9. `unconfirmed_relay_moves_on_to_the_next_entry`.
10. `two_failures_in_five_minutes_keep_the_relay` — 2 failed probes, then
    healthy again; no second `set`.
11. `three_failures_in_five_minutes_pick_another_relay` — and the failing one
    is not tried first.
12. `failures_spread_over_more_than_five_minutes_keep_the_relay` — one failed
    probe every 3 minutes (make the probe pass in between).
13. `relay_removed_from_the_list_is_replaced`.
14. `refresh_failure_keeps_the_relay_and_retries` — `fetches` grows on the
    backoff schedule; no new `set`.
15. `nothing_works_clears_the_map_and_retries_with_backoff` — `clears >= 1`,
    fetch count at about 5 s and 15 s (jitter is ±10 %), then a list that works → relay set.
16. `failed_repick_keeps_the_current_relay` (D-A4-12).
17. `cache_from_another_registry_is_ignored` (D-A4-3) — cache file written
    with another `registry_url`; first action is a fetch.
18. `max_age_zero_does_not_loop` (D-A4-8) — `fetches` after 30 s is at most 4.
19. `closed_endpoint_ends_the_task`.
20. `choice_is_written_to_the_cache_file` — load the file after the pick.
21. `saved_choice_off_the_saved_list_is_not_used_at_boot` (D-A4-12) — cache
    with chosen A and a fresh list `[B]`; A's info is healthy; `sets == [B]`,
    A is never probed. (Covers "refresh dropped A, then restart".)
22. `kept_relay_off_the_list_retries_on_the_backoff` (D-A4-12, D-A4-16) — A
    confirmed and healthy; a refresh drops A; the new list is `[B, C]`. B's
    info is healthy but its relay does not confirm; C's info answers `Down`.
    A stays current. Count **C's probes**, not fetches: only a re-pick try
    probes C (once per try, through the `tried` set), and a refresh never
    probes anything, so the count cannot be confused with refreshes. Assert
    C's probe count grows at about 5 s, 15 s and 35 s after the drop. Run the
    test twice, with `max_age_secs = 3600` and with `max_age_secs = 0`; the
    C counts must be the same in both runs (with `0` a refresh happens every
    10 s; a re-pick that skipped the backoff would show up as extra C
    probes). Also assert `sets` has B exactly once until 50 minutes after
    B's first failed confirm (the cool-down applies: A still works). Do not
    check at the 1-hour mark: the try schedule (5, 10, 20 … s, then every
    10 minutes, each ±10 % jitter) can put a try just after the cool-down
    ends, and the jitter would make an exact-hour check flaky. Finally list A again with a passing
    probe: C's count stops growing.
22b. `dead_kept_relay_gets_no_cool_down` (D-A4-16) — A confirmed; A's info
    goes `Down` for 3 health ticks, so a re-pick starts; list `[A, B]`; B's
    relay fails to confirm once, then confirms. A stays current after the
    first try (rollback). Assert B is set again at the next backoff step
    (about 5 s later), not 1 hour later, and becomes current.
23. `degraded_mode_warns_once_per_five_minutes` (D-A4-13) — registry always
    fails for 10 minutes. Capture logs the way three crates already do: a
    `MockWriter` (`Arc<Mutex<Vec<u8>>>`) behind
    `tracing_subscriber::fmt::layer().with_ansi(false).with_writer(…)`, made
    the thread default with `tracing::subscriber::set_default`. Copy the
    shortest one, `crates/fdae/src/compile/tests/trace_tests.rs:367-386`
    (others: `crates/app_supervisor/src/keys.rs:517-567`,
    `crates/control_plane/src/service/orchestration/tests/fdae_policy.rs:539-559`).
    Add `tracing-subscriber.workspace = true` to the substrate's
    `[dev-dependencies]`. The default `#[tokio::test]` runtime is
    single-threaded, so the spawned resolver task sees the thread default.
    There are already five copies of this writer in the workspace; run the
    duplication report on this file (§5.6). If it flags the copy, do not
    raise the baseline: drop the log capture and instead count warnings
    through the one helper that every resolver warning already goes through
    (D-A4-13), by making it return `true` when it logged at `warn` and having
    the test drive that helper with a paused clock. A shared log-capture
    helper for all crates is a separate clean-up (backlog row).
    Count lines with `WARN`: exactly 2 in 10 minutes (at 0 and at ~5 min).
24. `relay_set_outside_the_resolver_is_not_called_degraded` (D-A4-14) — fake
    `current` is set by the test, chosen is `None`; `clears == 0`; no
    degraded warning.

Note: AC-7 failover after 3 failures is tested only here, with paused time.
In a real run it needs at least 2 minutes of failing probes, too slow for the
e2e suite.

Use a `tempfile::tempdir()` for `cache_path`. If the file passes 800 lines,
split into `resolver/tests/{boot,keep}.rs` with `resolver/tests.rs` declaring
them.

Also unit-test `FailureWindow` and `Backoff` (cap, reset, jitter range) and
`ProbeVerdict::of` in the same file.

#### 8b. Integration tests, `crates/substrate/tests/home_relay_registry_e2e.rs` (new)

**Shared helpers first.** Move these from `home_relay_e2e.rs` into a new
`crates/substrate/tests/common/relay.rs` (declared in `common/mod.rs` as
`mod relay;` + `#[allow(unused_imports)] pub use relay::*;`, like `node`):
`spawn_relay`, `dead_relay`, `get_iroh_relay_url`, `wait_for_registry_relay`.
Update `home_relay_e2e.rs` to use them. Add to `relay.rs`:

```rust
/// A stand-in for a coordinator's `/v1/info`, on 127.0.0.1:0.
pub struct FakeInfo { pub url: String, state: Arc<Mutex<InfoMode>>, _task: JoinHandle<()> }
pub enum InfoMode { Healthy, AtCapacity, Unavailable /* 503 */ }
impl FakeInfo {
    pub async fn spawn(mode: InfoMode) -> Result<Self>
    pub fn set(&self, mode: InfoMode)
}

/// A parent registry on 127.0.0.1:0. `GET /coordinators` answers the list,
/// or 503 when it is `None`; only these requests are counted. `POST /register`
/// answers 200 and is not counted: S's own registry forwards every
/// registration to its parent (`registry.rs`, `propagate_registration`), so
/// counting it would break the "no list request" checks.
pub struct FakeListRegistry { pub url: String, list: Arc<Mutex<Option<CuratedList<CoordinatorEntry>>>>, requests: Arc<AtomicUsize>, _task: JoinHandle<()> }
impl FakeListRegistry {
    pub async fn spawn(list: Option<CuratedList<CoordinatorEntry>>) -> Result<Self>
    pub fn set(&self, list: Option<CuratedList<CoordinatorEntry>>)
    pub fn requests(&self) -> usize
}

/// A relay plus its fake info server, and the list entry for both.
pub struct ListedRelay { pub relay_url: RelayUrl, pub entry: CoordinatorEntry, pub info: FakeInfo, _server: Server }
pub async fn listed_relay(mode: InfoMode) -> Result<ListedRelay>
```

Bind every listener to `127.0.0.1:0` (allowed; no port literal). The
`FakeInfo` answer is a `CoordinatorInfo` with `relay: Some(RelayInfo {
online: true, .. })` and the status of the mode.

**The existing test that breaks.** `home_relay_e2e.rs`
`substrate_without_home_relay_skips_registration_until_relay_configured`
(line 139) sets `url = None`, and the harness always sets `registry_url`
(`node.rs:263-266`). After A4 that node runs the `Registry` source, and the
test's manual `home_relay().set()` fights the resolver. Setting
`registry_url = None` instead does not work: then no `EndpointPublisher` is
built (`runtime/router.rs`, `publisher` needs `registry_url` or the DHT), so
nothing is ever published and the test cannot pass. **Delete this test.** Its
claim (D4: no record before a confirmed relay, a record after) moves into new
test 6 `degraded_until_the_registry_answers`, which asserts both halves
through the resolver instead of a manual `set`. Record the move in the change
doc. The other three tests in that file set `url` (`Fixed` source; `run_fixed`
ends once the relay is confirmed), so their manual `set` calls stay valid.

**Topology for every test.** One substrate node S (the shared harness,
`SubstrateNode::builder()`), configured with:

```rust
.configure(move |c| {
    c.parent_coordinator.iroh.url = None;
    if let Some(reg) = c.roles.community_registry.as_mut() {
        reg.parent_registry_url = Some(fake_registry_url.clone());
    }
})
```

S's own registry has an empty list, so it forwards `GET /coordinators` to the
fake parent (this also covers forwarding end to end). S publishes its record
to its own registry, where the test reads it. Hold `common::serial_guard()`
for each test. S's own coordinator role runs its own relay; it is not listed.

The coordinator-list cache file is
`node.base_path().join("data/db/coordinators.json")` (the harness sets
`app_local_data_dir = base/data`; `db_dir` defaults to `db`).

Tests:

1. `picks_a_listed_relay_and_publishes_it` — two healthy listed relays,
   fake `max_age_secs = 3600`. Expected = `curated_list::rank(node.did(),
   entries)[0]`. `wait_for_registry_relay(expected, 30 s)`. The cache file has
   `chosen.url == expected`.
2. `skips_coordinators_that_are_full_or_down` — three entries: `AtCapacity`,
   `Unavailable`, `Healthy`. The record shows the healthy one, whatever the
   rank.
3. `restart_uses_the_saved_choice` — as test 1, note `requests()`,
   `node.restart()`, record shows the same relay, `requests()` unchanged.
4. `expired_list_is_used_when_the_registry_is_down` — two healthy entries,
   `max_age_secs = 1`. After the pick X: set X's info `Unavailable`, set the
   fake registry to `None` (503), sleep 2 s, `node.restart()`. The record
   shows the other entry.
5. `relay_that_leaves_the_list_is_replaced` — list `[A]`,
   `max_age_secs = 1`; record shows A; set list `[B]`; record shows B within
   40 s (refresh floor 10 s + confirm ≤ 15 s + publish).
6. `degraded_until_the_registry_answers` — fake starts with `None`; boot with
   `.skip_ready_wait()`; after 3 s the record lookup fails and
   `node.home_relay().current_url()` is `None`; set list `[A]`; record shows A
   within 40 s; `substrate_client.wait_for_ready(30 s)` passes.
7. `fixed_url_never_asks_the_registry` — do **not** clear `url` (harness
   default: S's own relay); parent = fake with one healthy entry; after boot,
   `requests() == 0`.
8. `relay_that_does_not_confirm_is_skipped` (change doc: "a relay that is
   down") — two entries with healthy `FakeInfo`; one relay URL comes from
   `dead_relay()` (accepts TCP, never answers). The record shows the live one,
   whatever the rank (worst case one 15 s confirm timeout first; allow 45 s).
9. `uses_a_real_coordinator_from_the_list` — boot a second harness node R
   first, with a known info port (`let [info_port] =
   common::alloc_ports::<1>()`, set
   `roles.coordinator.iroh.info_http_bind_address`). `alloc_ports` keeps a
   probe listener open on every port it returns (`PROBE_LISTENERS`,
   `tests/common/mod.rs`), and the harness frees only its own 4 ports
   (`node.rs`, `prepare_config`). So call
   `common::release_held_ports(&[info_port])` right before `boot()`, or the
   info server fails with `AddrInUse` (same pattern as
   `gateway_hostname_e2e.rs`, `spawn_tcp_backend`). R's default access is
   `"everyone"`. Set the fake list to one entry: `url = R.relay_url()`,
   `info_url = http://127.0.0.1:<info port>`. Boot S. The record shows R's
   relay. This runs the real `info_url` + `INFO_PATH` + `relay.online` path.
10. `coordinator_role_never_asks_for_the_list` (AC-16) — S with `url = None`,
    `substrate.communication_interfaces = []` (so the router builds no
    endpoint and no resolver runs), the coordinator role on, parent = fake
    with one healthy entry; allocate S's info port as in test 9 and call
    `common::release_held_ports(&[info_port])` right before boot; boot with
    `.skip_ready_wait().no_owner_client()`;
    after 5 s `requests() == 0`, and S's coordinator `/v1/info` (known info
    port) reports `relay_url` = S's own relay. If someone wires the list into
    the coordinator role, the count is no longer 0.
    Honest limit: AC-16 is mostly guaranteed by structure (the resolver is a
    private module of `syneroym-substrate`, and the coordinator crates do not
    depend on that crate). The old idea of asserting
    `parent_coordinator_url.is_none()` proves nothing, because that field is
    copied from the config `url` (`coordinator.rs:254`); it is dropped.

Never call `node.home_relay().set(...)` in these tests: the resolver owns the
handle and would undo it.

### Step 9 — what is not changed, on purpose

- `crates/router/src/route_handler.rs`: the field `_parent_relay_url`
  (declared at 106; filled from the config at 370 and 414; the
  `new_coordinator` parameter at 442, stored at 457) stores `parent_coordinator.iroh.url` and nothing reads
  it. After A4 it is `None` for a resolved substrate. Leave it (see §4.9).
- Coordinator roles (`crates/coordinator_iroh/src/coordinator.rs:106`,
  `crates/coordinator_webrtc/src/coordinator.rs:58`) keep reading only
  `parent_coordinator.iroh.url` (AC-16).
- `publish.rs`: the watch channel already publishes on every confirmed
  change (AC-8). No change.

## 4. Things that are ambiguous or stale in task.md and the change docs

Each item gives the problem, what this plan does, and whether the owner must
confirm it first.

1. **"Connection failures" in AC-7 have no source (owner check).** AC-7 and
   task.md say "fail over after 3 failures in 5 minutes" but do not say what
   a failure is. iroh 0.97 exposes no event or counter for "the home relay
   connection failed" (`Endpoint` has `watch_addr`, `relay_map`, `metrics`;
   `socket::Metrics` has relay path counters, none for connect failures).
   `watch_addr` may also keep a dead home relay, because iroh keeps its old
   home relay when no other relay answers. Plan: a failure is a failed info
   probe of the chosen entry, every 60 s (D-A4-4). Alternative: watch
   `socket::Metrics` counters — not reliable for this. Load: one small HTTP
   request per substrate per minute to its coordinator.
   **A failure mode this does not catch.** iroh confirms a relay before the
   relay checks access (`crates/router/src/home_relay.rs:9-12`; the change
   doc's Deviations). If a registry wrongly lists a relay whose `access` is
   not `"everyone"`, the node confirms it, publishes it, its info probes stay
   healthy, and it stays unreachable through the relay without ever failing
   over. Neither the info probe nor anything iroh 0.97 exposes can see this.
   Detecting it needs a reachability check through the relay (for example
   the node dialing its own record through the relay), which is new work.
   Plan: not handled in A4; add a backlog row (§5.4). The registry operator
   rule (list only coordinators with `access = "everyone"`) stays the guard.
2. **AC-9 and AC-12 disagree about an old cache (owner check).** AC-9 allows a
   saved list to be used when `registry_url` is not set. AC-12 says the
   substrate never contacts a relay that a *configured* registry did not list.
   A list saved from a registry that the operator has since removed or
   changed breaks AC-12. Plan: bind the cache to its registry URL (D-A4-3), so
   with no `registry_url` there is no usable cache. Then AC-9's "no saved
   choice or list" clause adds nothing when `registry_url` is unset; reword
   AC-9 at close-out.
2b. **A relay that leaves the list (owner check, D-A4-12).** AC-7 says pick
   again when the choice is removed; AC-12 says never contact a relay the
   configured registry did not list. Neither says what to do when nothing
   else works. Plan: never make a *new* connection to a relay off the list
   (the boot path checks the saved list, so "refresh drops A, then restart"
   does not reconnect to A); but do not drop a relay that is connected and
   working, and keep trying to replace it on the 5 s … 10 min backoff. Reason:
   dropping it makes the node unreachable, and the relay has already seen
   this node, so AC-12's privacy aim gains nothing. Alternative (stricter):
   drop to degraded mode at once. That needs a new `HomeRelay` method that
   removes a *confirmed* relay and sends `None` on the watch channel, and the
   last published record would still name the dropped relay until it
   expires. Reword AC-12 at close-out: "…did not list. A relay removed from
   the list at run time is kept only until a listed replacement is
   confirmed."
   **Risk of the replacement tries.** Each try may call `set(B)` on an entry
   whose `/v1/info` answers but whose relay does not confirm. During that
   switch A is out of the map for up to 15 s, so the node may be unreachable
   through a relay; then `set` rolls back to A, and that rollback is not
   confirmed (D5; it only logs a warning if iroh does not list A). Without a
   limit this repeats on every backoff step, in a long outage once every 10
   minutes (up to 15 s in 600 s, about 2.5 % of the time). The plan limits
   it with a cool-down: an entry that failed to confirm is not tried again
   for 1 hour while a *working* relay is kept (D-A4-16), so the cost is at
   most one 15 s gap per bad entry per hour. When the kept relay is failing
   its probes there is no cool-down: it is probably dead, a failed switch
   costs nothing, and waiting would only stretch the outage. Options for the owner: accept the
   cool-down; or never try a switch while a working relay is kept, and
   replace it only when it fails 3 probes (simpler, but a relay off the list
   could then stay for as long as it works); or the stricter "drop at once"
   above.
3. **AC-5 versus a fresh list where nothing works (owner check).** Read
   literally, AC-5 forbids asking the registry until `max_age_secs` (default
   86,400 s) passes, so a fresh list of dead relays leaves the node without a
   relay for up to a day. AC-9's narrow definition would not even call this
   "degraded". Plan: ask the registry when the fresh list gives no relay
   (D-A4-6), log it as degraded mode, retry with backoff.
4. **Saved choice `at_capacity` at boot (owner check).** AC-2 step 2 says
   "still answers `info_url`", AC-6 says skip `at_capacity`. After a restart
   the old connection may still be counted until the relay sees it close, so
   the node's own ghost can make its relay look full. Plan: keep a full saved
   choice; skip full entries only when picking new (D-A4-5).
5. **AC-16 and a node that runs both roles.** AC-16 talks about the
   coordinator role's own endpoint. The substrate router on the same node
   still resolves its home relay from the list when its `url` is unset (the
   test harness runs every node like this). Plan: that is intended; the e2e
   test `coordinator_role_never_asks_for_the_list` runs the coordinator role
   with no router endpoint and checks that no list request is made. Confirm
   if you meant "a node with a coordinator role never resolves".
6. **Bad `substrate.registry_url` (owner check).** Today a bad value only
   makes registry calls fail. With A4 it stops startup when `url` is unset
   and `"iroh"` is in `communication_interfaces` (D-A4-2), as D2 does for
   `url`. A WebRTC-only node is not affected. Alternative: degrade with a warning. The
   plan prefers failing at startup, because a typo would otherwise leave the
   node unreachable with only a log line.
7. **Forward timeouts change registry behaviour (owner check).** The A2 note
   asks A4 to "design hop-budgeted or decreasing forward timeouts". The plan
   changes `curated_list::fetch` for every caller, including registry
   forwarding (D-A4-9). A registry that is the first forward now waits 30 s,
   not 10 s, for its parent.
8. **Stale facts in task.md and the change doc.**
   - task.md header still says "Status: planning (2026-10-11)".
   - status.md marks A3 "Done", but A3 is not in `main` (see §0, Branch).
   - change.md "Boot and runtime change" cites `connection_router.rs:81` and
     `runtime/router.rs:220`; after A1 the endpoint is built at
     `connection_router.rs:82-100` and the record publisher reads the watch
     channel (`publish.rs:154-175`). The relay is no longer "read once".
   - change.md "Crate homes" lists `route_handler.rs:370` as a reader of
     `url`; it only stores it in a dead field (item 9).
   - Comments that become false: `IrohParentConfig.url`'s doc comment
     (`crates/core/src/config/base.rs:153-155`, "When it is not set, the
     endpoint starts with an empty relay map …") and the same text in
     `crates/substrate/config.sample.toml:94-95`. Fix both in A4 (§5.3).
   - deferred-backlog row "Degraded node stays degraded until restart" says
     "Until slices A4 and B3 land"; after A4 only a node with neither `url`
     nor `registry_url` stays degraded, which is by design.
9. **Dead field `_parent_relay_url`** (`route_handler.rs`). It holds the
   config `url`, is never read, and after A4 does not name the home relay of a
   resolved node. Out of scope; removing it touches `new_coordinator`'s
   public signature in two coordinator crates. Recommend a separate small
   clean-up; add a backlog row if the owner agrees.
10. **`HomeRelay::set` leaves a failed first relay in the map**, which AC-9
    does not allow in degraded mode. The plan adds `clear_unconfirmed`
    (D-A4-11). Known iroh 0.97 limit (from A1): after the last relay is
    removed, `Endpoint::addr` may still list it. This does not reach the
    record, which follows the watch channel.
11. **B3 and who owns the `HomeRelay` handle (owner check, D-A4-15).** The
    passport doc lets `roymctl substrate home-relay set --coordinator <url>`
    hand over a token through the control plane, and H-1 says the relay
    then admits the node "with no restart" (`operator-passport/change.md`,
    H-1 at ~line 60 and the command at ~line 143). On a node whose
    source is `Registry` (or `None`), that command would fight the resolver,
    and `connect_home_relay` returns no handle to stop it. A4 does not build
    the stop path (no user yet, and its shape depends on B3's control-plane
    call). A4 writes the contract into the passport doc and keeps the B3 half
    of the two backlog rows open. Recommended contract for B3: the passport
    step applies to the `Fixed` source; on another source the command either
    refuses, or stops the resolver through a cancellation token that is
    checked between steps, never while `HomeRelay::set` runs.

## 5. Completion steps

1. **Imports pass** over every edited file (`AGENTS.md`: types by `use`,
   functions through their parent module, no inline `a::b::c` paths).
2. **Change doc** `docs/planning/changes/registry-listed-coordinators/change.md`:
   - Fill the Test column of AC-2, AC-4 (substrate part), AC-5 to AC-9,
     AC-11, AC-12, AC-16 with the test names of Step 8.
   - Tick the two open Tasks ("Substrate: background resolve…" and
     "Integration tests…").
   - Add `D-A4-1` … `D-A4-16` to `## Deviations`, plus any difference you
     made from this plan, and: "the test
     `substrate_without_home_relay_skips_registration_until_relay_configured`
     is replaced by `degraded_until_the_registry_answers`" (Step 8b).
   - Reword AC-9 if §4.2 is accepted, AC-12 for §4.2b, and AC-7 for §4.1.
   - In `docs/planning/changes/operator-passport/change.md`, add the B3
     contract of D-A4-15 under its `home-relay set` design.
   - Set `status: done` and check `living-docs-touched`.
3. **Code comments and sample config that become false:**
   `crates/core/src/config/base.rs:153-155` (`IrohParentConfig.url` doc:
   say that without `url` the substrate picks a relay from
   `substrate.registry_url`'s coordinator list, and is in degraded mode only
   when that gives none) and `crates/substrate/config.sample.toml:94-95` (same
   meaning, as a TOML comment).
   **Living docs** (update so they describe the result; link ADRs only).
   Search each file for `parent_coordinator.iroh.url`, "static", "relay URL",
   "caches" and "degraded" rather than trusting the line numbers below:
   - `docs/system-architecture.md`: the "Envisioned" note that says "a relay
     is a URL that an operator writes into the config of each substrate"
     (~292), the bootstrap "Envisioned" note "no list of relays exists, and no
     substrate caches one" (~392), the home-relay paragraph (~line 256), the
     `[parent_coordinator.iroh] url` row of the config table (~357: "When
     omitted and `substrate.registry_url` is set, the substrate picks a relay
     from that registry's coordinator list"), the "Default relay" paragraph
     (~382), the "Envisioned" note and the Rp/Sz bullet (~1751-1753: now
     built), and the curated-list paragraph (~2105: remove "No substrate uses
     the list yet", add the resolve order, the cache file, the 60 s health
     probe, the per-hop timeouts).
   - `docs/system-requirements-spec.md` (~154, ~160, ~298-299, ~515, ~517,
     ~534, ~582): "substrates configure static … relay URLs" and similar;
     the relay now comes from `url` or from the registry list. Keep the
     "Envisioned" parts that are still not built (bootstrap servers, DNS
     names, the 24-hour outage test).
   - `docs/TERMINOLOGY.md`: "Degraded mode" — when it happens now (no `url`,
     and no relay from the list).
   - `docs/developer-guide.md`: if it lists files under `storage.db_dir`, add
     `coordinators.json`.
   - `docs/planning/traceability-matrix.md`: `[TOP-REG]` note — substrates
     now use the coordinator list.
4. **Deferred backlog** (`docs/planning/deferred-backlog.md`):
   - Move to "Recently resolved": "Substrate must not treat an empty
     coordinator list as usable", "Compare relay URLs only through
     `normalize_url`", "Curated-list forward timeout across multi-hop
     chains".
   - **Narrow, do not close**, "Degraded node stays degraded until restart"
     and "`run_fixed` background task cancellation and single ownership":
     rewrite each to its B3 half (a run-time `home-relay set` on a node with
     the `Registry` or no source must stop or refuse the resolver; D-A4-15),
     target B3.
   - Add: "A listed relay that refuses this node is never left" (§4.1; needs
     a reachability check through the relay; target `TBD`).
   - Add: "Home relay health is read from `/v1/info`, not from the relay
     connection" (if §4.1 stays; iroh 0.97 has no signal).
   - Add the dead field of §4.9 if the owner agrees.
   - If test 23 fell back to the helper count (Step 8a), add "No shared
     log-capture test helper; five copies of `MockWriter` in three crates".
5. **Milestone docs**: `status.md` A4 row (status, implemented by, evidence:
   test files and pass counts, `mise run verify`). Do not edit task.md beyond
   fixing its stale status line if the owner asks.
6. **Duplication check** on changed files (recipe in `AGENTS.md`, Context
   Budget). `FakeInfo`/`FakeListRegistry` must not copy existing helpers;
   `crates/community_registry/src/curated_list/tests.rs` has its own list
   server — reuse its shape only if it is already public to substrate tests
   (it is not; keep the new one in `tests/common/relay.rs`).
7. **Ratchets**: if a gate says a number got better, record the new value in
   the same change (`AGENTS.md`, Mandatory Ratchet Update).
8. **Verify**: run `mise run verify` in the background (about 20 minutes) and
   read the summary. Every gate must pass with no `--skip`. The e2e tests
   need the sandbox off (project memory).
9. Do not commit unless the user asks. On a feature branch, commits are
   allowed when asked; suggested split: `feat(core): budget list forward
   timeouts per hop`, `feat(router): add home relay clear and close wait`,
   `feat(substrate): resolve home relay from registry`, `docs: close out
   registry-listed coordinators`.
