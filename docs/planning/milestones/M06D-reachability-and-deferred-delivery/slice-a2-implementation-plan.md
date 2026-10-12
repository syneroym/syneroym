# M06D slice A2 — Curated lists and ranking: implementation plan

*Reader: the developer (or AI) who implements slice A2. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  (no "M06D", "A2", "X2", "X3", "AC-13" in code, comments or test names; ADR
  ids are fine), simple English in comments and docs, the 800-line file cap
  and the module layout rule (`foo.rs` beside `foo/`, never `mod.rs`).
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 (rows X2 and X3), §3 (slice A2, and C7, which reuses this work).
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-3, AC-4,
  AC-13, AC-15, the design parts "Registry side" and "URL form and ranking",
  and `## Deviations`.
- `docs/planning/changes/service-mailbox/change.md`: K-3 and the paragraph
  on `[[roles.community_registry.mailboxes]]` (the second list, built in C7).
  Do not build it. Only make sure the generic code fits it.

**Branch.** Slice A1 is on `feat/m06d-a1-implementation` and is not merged
to `main` yet. A2 builds on A1's code. Create the A2 branch from the A1
branch:

```bash
git fetch origin feat/m06d-a1-implementation
git checkout -b feat/m06d-a2-curated-lists origin/feat/m06d-a1-implementation
```

When A1 merges, merge `main` into the A2 branch (do not rebase a pushed
branch). Push with `git push -u origin feat/m06d-a2-curated-lists`. Do not
open a pull request unless the user asks. Commit messages follow the
Conventional Commits and 50/72 rules in `AGENTS.md`.

### Terms used in this plan

- **Curated list.** A list of entries that a registry operator writes by hand
  into the registry config. The registry serves it over HTTP. The coordinator
  list (open Iroh relays) is the first one. The mailbox list (slice C7) is the
  second one.
- **Entry id.** The value that makes an entry unique in its list. For a
  coordinator it is the normalised relay URL. For a mailbox it will be the
  mailbox service id.
- **Chooser.** The party that ranks a list: the substrate DID for the
  coordinator list, the service id for the mailbox list.
- **Rendezvous ranking.** Each chooser sorts the entries by
  `SHA-256(len(chooser) || chooser || len(id) || id)`, smallest first. When an
  entry leaves the list, only the choosers that had picked it move.
- **Hop.** One forward of a list request from a registry to its parent
  registry (`parent_registry_url`).

## 1. What this slice delivers

1. `crates/core`: a new module `curated_list` with
   - the URL normaliser,
   - the rank key and the rank function, with a fixed test vector,
   - the generic list trait `ListEntry`, the wire type `CuratedList<E>`, the
     entry check `check_entries`, and the client call `fetch`,
   - the first entry type, `CoordinatorEntry`.
2. `crates/core` config: `[[roles.community_registry.coordinators]]` and
   `[roles.community_registry] list_max_age_secs`.
3. `crates/community_registry`: a generic list route with parent forwarding
   and a hop limit. Its first instance is
   `GET /coordinators?transport=iroh-relay`. The registry refuses to start with
   a bad entry or a duplicate.
4. Tests for AC-3, AC-4, AC-13 and AC-15.
5. Living docs, change doc, backlog and status updates.

Not in this slice: the substrate resolver, the cache file, `/v1/info` probes
(A3, A4), the mailbox list (C7), any `roymctl` command.

**Done rule (from task.md).** After this slice, a second list must need only:
an entry type with an `impl ListEntry`, one config field, and one `.merge(...)`
line in the registry. Step 6 checks this with a test-only second entry type.

## 2. Decisions made in this plan

The change docs leave these open. Each one is a choice; record each in the
change doc `## Deviations` (see the completion steps).

**D-A2-1. Name of the cache-time setting.** task.md and the change doc name
the response field `max_age_secs` but no config key. An array of tables
(`[[roles.community_registry.coordinators]]`) cannot carry it. Use one setting
for all lists: `[roles.community_registry] list_max_age_secs`, default `86400`
(the value in the change doc example). The mailbox list uses the same value.

**D-A2-2. What "3 hops" means.** The request carries `hops` (a query
parameter, `u8`, default `0`). It counts the forwards already made. A registry
whose own entries (after the filter) are empty forwards only when it has a
parent and `hops < 3`. It forwards with `hops + 1`. So at most 3 forwards
happen, and at most 4 registries see one client request. A registry that may
not forward any more answers its own empty list with `200`.

**D-A2-3. A parent that fails.** When a registry must forward and the parent
cannot be reached, answers with a non-2xx status, or sends a body that does
not parse, the registry answers `502 Bad Gateway` with a short text body. It
does not answer an empty list. Reason: an empty `200` with
`max_age_secs = 86400` would make a substrate cache "no relays" for a day.
With `502`, the substrate's refresh fails and it keeps its old list (AC-5).

**D-A2-4. A parent's answer is passed on as it is.** The registry does not
re-check, re-normalise or cache it, and it keeps the parent's `max_age_secs`.
The client ranks with `rank`, which checks and normalises every entry and
drops bad ones, so a bad parent entry cannot reach a choice. No cache: each
request to a registry with an empty list costs one request to its parent.
Record this in the deferred backlog (step 9).

**D-A2-5. The URL normaliser is strict.** The change doc says: lower-case
scheme and host, drop a default port, drop a trailing slash. In addition, the
normaliser rejects:
- a scheme other than `http` or `https`,
- a URL with no host,
- a user name or password,
- a query or a fragment.
A relay URL with these parts has no meaning, and a query would make two
spellings of one relay rank differently. `url::Url` already lower-cases the
scheme and host, applies IDNA, and drops the default port while it parses.
The normaliser then removes exactly one trailing `/` from the serialised URL.
A path is kept (`https://h/base/` becomes `https://h/base`).

**D-A2-6. Both URLs of a coordinator entry are normalised.** `url` is the
entry id. `info_url` is checked and normalised the same way, but it is not
part of the id. The registry serves both in normalised form.

**D-A2-7. The response key depends on the list.** The change doc fixes the
coordinator response as `{ "max_age_secs": ..., "coordinators": [...] }`. So
the generic wire type `CuratedList<E>` has hand-written `Serialize` and
`Deserialize` that use `E::LIST_KEY` as the key of the array. The same
constant is the route path (`/coordinators`). Unknown keys in a response are
ignored, so a newer parent can add fields.

**D-A2-8. `transport` is required and strict.** `GET /coordinators` without
`transport`, or with an unknown value, answers `400` (axum's `Query`
rejection). In config, an entry with an unknown `transport` fails to parse.
The only value is `iroh-relay`.

**D-A2-9. Entry types do not deny unknown fields.** `CoordinatorEntry` is
used both in the config and on the wire. On the wire it must accept fields a
newer registry adds (for example a later `bootstrap_url`). A typo in a
required config field still fails, because the field is then missing.

**D-A2-10. An empty list is served with the normal `max_age_secs`.** This
slice does not give an empty answer a shorter cache time. Slice A4 must not
treat an empty list as a usable saved list (it would block AC-9's retry with
backoff). Write this as a note for A4 in the change doc (completion step 2).

**D-A2-11. No new crate dependency.** `crates/core` already depends on
`reqwest` and `sha2`. Use `reqwest::Url` (a re-export of `url::Url`) for
parsing. reqwest 0.13 is built without its `query` feature in this workspace
(`Cargo.lock` shows no `serde_urlencoded` under `reqwest 0.13.5`), so
`RequestBuilder::query` does not exist. Build the query with
`Url::query_pairs_mut()` instead. On the server side, axum's `Query`
extractor is available (axum default features are on).

## 3. Step-by-step changes

Line numbers are from `feat/m06d-a1-implementation` at commit `df72c7a`.
Check them again; they move.

### Step 1 — `crates/core`: the `curated_list` module

New files (module layout rule: `curated_list.rs` beside `curated_list/`):

```text
crates/core/src/curated_list.rs              trait, wire type, check, fetch, normaliser
crates/core/src/curated_list/rank.rs         rank_key, rank
crates/core/src/curated_list/coordinator.rs  CoordinatorEntry and friends
crates/core/src/curated_list/tests.rs        unit tests (step 5)
```

Add `pub mod curated_list;` to `crates/core/src/lib.rs` (keep the list in
alphabetical order: after `config`).

#### 1a. `curated_list.rs`

```rust
//! Curated lists: lists of entries that a registry operator writes into the
//! registry config, and that the registry serves over HTTP. A client ranks a
//! list with `rank` to pick one entry.

mod coordinator;
mod rank;

pub use coordinator::{CoordinatorEntry, CoordinatorFilter, CoordinatorTransport};
pub use rank::{rank, rank_key};

/// The query parameter that counts forwards between registries.
pub const HOPS_PARAM: &str = "hops";
/// A registry forwards a list request to its parent only while the request
/// has made fewer forwards than this. It stops loops between registries.
pub const MAX_LIST_HOPS: u8 = 3;
/// How long a list request waits. Same reason and value as the registry
/// client's own timeout: an unanswering registry must not stall the caller.
pub const LIST_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// One kind of entry in a curated list.
pub trait ListEntry: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The route (`GET /<LIST_KEY>`), the JSON key of the entry array, and the
    /// config key under `[roles.community_registry]`.
    const LIST_KEY: &'static str;
    /// The query parameters a client sends to choose entries.
    type Filter: DeserializeOwned + Send + Sync + 'static;

    /// Whether this entry answers `filter`.
    fn matches(&self, filter: &Self::Filter) -> bool;
    /// `filter` as query pairs, to forward a request to a parent registry.
    fn filter_pairs(filter: &Self::Filter) -> Vec<(&'static str, String)>;
    /// Checks the entry, normalises its fields in place, and returns its id.
    ///
    /// # Errors
    /// When a field is not valid.
    fn check(&mut self) -> anyhow::Result<String>;
}

/// The body of `GET /<LIST_KEY>`:
/// `{ "max_age_secs": <u64>, "<LIST_KEY>": [ ... ] }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratedList<E> {
    /// How long a client may cache this answer.
    pub max_age_secs: u64,
    pub entries: Vec<E>,
}
```

Hand-written serde for `CuratedList<E: ListEntry>` (D-A2-7):

```rust
impl<E: ListEntry> Serialize for CuratedList<E> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("max_age_secs", &self.max_age_secs)?;
        map.serialize_entry(E::LIST_KEY, &self.entries)?;
        map.end()
    }
}

impl<'de, E: ListEntry> Deserialize<'de> for CuratedList<E> {
    // deserialize_map with a visitor (PhantomData<E>):
    //   while let Some(key) = map.next_key::<String>()? {
    //       if key == "max_age_secs"   { set once, else duplicate_field error }
    //       else if key == E::LIST_KEY { set once, else duplicate_field error }
    //       else { map.next_value::<de::IgnoredAny>()?; }
    //   }
    //   both missing -> de::Error::missing_field("max_age_secs") / (E::LIST_KEY)
}
```

`de::Error::missing_field` and `duplicate_field` take `&'static str`;
`E::LIST_KEY` is `&'static str`, so this works.

The entry check, used by the registry at startup (AC-13):

```rust
/// Checks and normalises every entry, and refuses duplicate ids.
/// The error names the config key and the entry index, for example
/// `roles.community_registry.coordinators[2]`.
///
/// # Errors
/// On the first bad entry or duplicate.
pub fn check_entries<E: ListEntry>(entries: &mut [E]) -> anyhow::Result<()> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter_mut().enumerate() {
        let id = entry
            .check()
            .with_context(|| format!("roles.community_registry.{}[{i}]", E::LIST_KEY))?;
        if let Some(first) = seen.insert(id.clone(), i) {
            anyhow::bail!(
                "roles.community_registry.{}[{i}] is a duplicate of entry [{first}] ({id})",
                E::LIST_KEY
            );
        }
    }
    Ok(())
}
```

The client call. The registry uses it to forward now; slice A4 uses it to
fetch.

```rust
/// Asks the registry at `registry_url` for its list.
/// `hops` is the number of forwards already made; a client sends 0.
///
/// # Errors
/// When the registry cannot be reached, answers with a non-2xx status, or
/// sends a body that does not parse.
pub async fn fetch<E: ListEntry>(
    client: &reqwest::Client,
    registry_url: &str,
    filter: &E::Filter,
    hops: u8,
) -> anyhow::Result<CuratedList<E>> {
    let base = registry_url.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base}/{}", E::LIST_KEY))
        .with_context(|| format!("registry URL {registry_url:?}"))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in E::filter_pairs(filter) { q.append_pair(k, &v); }
        q.append_pair(HOPS_PARAM, &hops.to_string());
    }
    let list = client.get(url).send().await?.error_for_status()?
        .json::<CuratedList<E>>().await?;
    Ok(list)
}
```

`format!("{base}/...")` matches how `RegistryClient::lookup` builds its URL
(`crates/core/src/dht_registry/client.rs`). Do not use `Url::join`: it drops
the last path segment of a base without a trailing slash.

The normaliser (D-A2-5):

```rust
/// The normal form of a relay or info URL: lower-case scheme and host, no
/// default port, no trailing slash. Ranking and the duplicate check use this
/// form, because `x` and `x/` must be the same entry.
///
/// # Errors
/// When `raw` is not an `http` or `https` URL with a host, or has a user,
/// password, query or fragment.
pub fn normalize_url(raw: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw).with_context(|| format!("{raw:?} is not a valid URL"))?;
    ensure!(matches!(url.scheme(), "http" | "https"), "{raw:?}: scheme must be http or https");
    ensure!(url.host_str().is_some(), "{raw:?}: URL has no host");
    ensure!(url.username().is_empty() && url.password().is_none(), "{raw:?}: URL has a user or password");
    ensure!(url.query().is_none() && url.fragment().is_none(), "{raw:?}: URL has a query or fragment");
    let s = url.as_str();
    Ok(s.strip_suffix('/').unwrap_or(s).to_owned())
}
```

Imports (import rule: types by `use`, functions through their parent module):
`std::{collections::HashMap, fmt, marker::PhantomData, time::Duration}`,
`anyhow::{Context, ensure}`, `reqwest::Url`,
`serde::{Deserialize, Serialize, Serializer, de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor}, ser::SerializeMap}`.

#### 1b. `curated_list/rank.rs`

```rust
//! Rendezvous ranking. Each chooser gets its own stable order of a list.
//! When an entry leaves the list, only the choosers that picked it move.

/// `SHA-256(len(chooser) || chooser || len(id) || id)`, with each `len` a
/// 4-byte big-endian byte count. Not Rust's default hasher: its output may
/// change between compiler versions.
pub fn rank_key(chooser: &str, id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in [chooser, id] {
        // A string of 4 GiB or more cannot be an id; saturating keeps this total.
        let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
        hasher.update(len.to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().into()
}

/// Checks and normalises each entry, drops the bad ones and the duplicates,
/// and sorts the rest by `rank_key(chooser, id)`, smallest first. The first
/// entry is the choice; the rest is the failover order.
pub fn rank<E: ListEntry>(chooser: &str, entries: impl IntoIterator<Item = E>) -> Vec<E> {
    let mut keyed: Vec<([u8; 32], String, E)> = entries
        .into_iter()
        .filter_map(|mut e| match e.check() {
            Ok(id) => Some((rank_key(chooser, &id), id, e)),
            Err(err) => { tracing::debug!(error = %err, "skipping a bad list entry"); None }
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    // Equal ids give equal keys, so duplicates are next to each other.
    keyed.dedup_by(|a, b| a.1 == b.1);
    keyed.into_iter().map(|(_, _, e)| e).collect()
}
```

#### 1c. `curated_list/coordinator.rs`

```rust
//! The coordinator list: open Iroh relays that a substrate may use as its
//! home relay.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoordinatorTransport {
    /// An Iroh relay.
    IrohRelay,
}

impl CoordinatorTransport {
    pub const fn as_str(self) -> &'static str {
        match self { Self::IrohRelay => "iroh-relay" }
    }
}

/// One entry of `[[roles.community_registry.coordinators]]`, and of the
/// `GET /coordinators` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinatorEntry {
    pub transport: CoordinatorTransport,
    /// The relay URL. The entry id.
    pub url: String,
    /// Where `GET /v1/info` of the coordinator is served. It has its own port.
    pub info_url: String,
    /// Who runs the coordinator. Shown to people; not checked.
    pub operator: String,
}

/// `GET /coordinators?transport=iroh-relay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct CoordinatorFilter {
    pub transport: CoordinatorTransport,
}

impl ListEntry for CoordinatorEntry {
    const LIST_KEY: &'static str = "coordinators";
    type Filter = CoordinatorFilter;

    fn matches(&self, filter: &CoordinatorFilter) -> bool { self.transport == filter.transport }

    fn filter_pairs(filter: &CoordinatorFilter) -> Vec<(&'static str, String)> {
        vec![("transport", filter.transport.as_str().to_owned())]
    }

    fn check(&mut self) -> anyhow::Result<String> {
        self.url = curated_list::normalize_url(&self.url).context("url")?;
        self.info_url = curated_list::normalize_url(&self.info_url).context("info_url")?;
        Ok(self.url.clone())
    }
}
```

No `#[serde(deny_unknown_fields)]` (D-A2-9).

### Step 2 — `crates/core` config

File `crates/core/src/config/roles.rs`, `ServiceRegistryRole` (~l.31-47):

```rust
const fn default_list_max_age_secs() -> u64 { 86_400 }

pub struct ServiceRegistryRole {
    pub access: AccessControl,
    pub http_bind_address: String,
    pub parent_registry_url: Option<String>,
    /// `[[roles.community_registry.coordinators]]`: open Iroh relays this
    /// registry lists. Only list a coordinator whose
    /// `[roles.coordinator] access` is "everyone".
    pub coordinators: Vec<CoordinatorEntry>,
    /// How long a client may cache any curated list of this registry.
    pub list_max_age_secs: u64,
}
```

`Default`: `coordinators: Vec::new()`, `list_max_age_secs: default_list_max_age_secs()`.
The struct already has `#[serde(default)]`, so old configs still parse.
Import `crate::curated_list::CoordinatorEntry`.

**Every struct literal of `ServiceRegistryRole` that lists all fields must
get `..Default::default()`.** At planning time these five do not compile
after the change:

| File | Line |
| --- | --- |
| `crates/smoke-tests/src/main.rs` | ~86 |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs` | ~285, ~507, ~523 |
| `crates/community_registry/src/registry/tests.rs` | ~50 |

These already use `..Default::default()` and need no change:
`crates/substrate/tests/basic_lifecycle.rs:231`,
`crates/substrate/tests/podman_lifecycle.rs:85`,
`crates/substrate/tests/common/node.rs:247`,
`crates/substrate/tests/common/mod.rs:279`,
`crates/coordinator_webrtc/src/bootstrap.rs:259`. Find all with
`grep -rn 'ServiceRegistryRole {' crates apps`.

Also update `crates/substrate/config.sample.toml` (~l.153): after
`http_bind_address`, add a commented example:

```toml
# How long a client may cache a curated list of this registry (seconds).
# list_max_age_secs = 86400
#
# Open Iroh relays that substrates without their own relay may use.
# Only list a coordinator whose [roles.coordinator] access is "everyone".
# [[roles.community_registry.coordinators]]
# transport = "iroh-relay"
# url = "https://relay.example.org"
# info_url = "https://relay.example.org:7974"
# operator = "Example Org"
```

### Step 3 — `crates/community_registry`: the generic list route

New files:

```text
crates/community_registry/src/curated_list.rs
crates/community_registry/src/curated_list/tests.rs
```

Add `mod curated_list;` to `crates/community_registry/src/lib.rs`.

```rust
//! The HTTP side of curated lists: one generic route per list, with
//! forwarding to the parent registry when this registry lists nothing.

struct ListState<E> {
    entries: Vec<E>,
    max_age_secs: u64,
    parent_registry_url: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct HopQuery {
    #[serde(default)]
    hops: u8,
}

/// The routes of every curated list of this registry. A new list adds one
/// `.merge(list_route(...)?)` line here.
///
/// # Errors
/// When an entry is bad or a duplicate. The registry then does not start.
pub(crate) fn router(role: &ServiceRegistryRole) -> anyhow::Result<Router> {
    let client = reqwest::Client::builder()
        .timeout(curated_list::LIST_FETCH_TIMEOUT)
        .build()
        .context("building the curated-list HTTP client")?;
    Ok(Router::new().merge(list_route(role.coordinators.clone(), role, client)?))
}

fn list_route<E: ListEntry>(
    mut entries: Vec<E>,
    role: &ServiceRegistryRole,
    client: reqwest::Client,
) -> anyhow::Result<Router> {
    curated_list::check_entries(&mut entries)?;
    info!(list = E::LIST_KEY, count = entries.len(), "serving curated list");
    let state = Arc::new(ListState {
        entries,
        max_age_secs: role.list_max_age_secs,
        parent_registry_url: role.parent_registry_url.clone(),
        client,
    });
    Ok(Router::new().route(&format!("/{}", E::LIST_KEY), get(serve_list::<E>)).with_state(state))
}
```

The handler. Use **two** `Query` extractors, not one struct with
`#[serde(flatten)]`: `serde_urlencoded` with `flatten` reads every value as a
string, so `hops: u8` would fail to parse. Each `Query` reads the whole query
string and ignores keys it does not know.

```rust
async fn serve_list<E: ListEntry>(
    State(state): State<Arc<ListState<E>>>,
    Query(filter): Query<E::Filter>,
    Query(hop): Query<HopQuery>,
) -> Result<Json<CuratedList<E>>, (StatusCode, String)> {
    let entries: Vec<E> = state.entries.iter().filter(|e| e.matches(&filter)).cloned().collect();
    let own = CuratedList { max_age_secs: state.max_age_secs, entries };
    if !own.entries.is_empty() { return Ok(Json(own)); }
    let Some(parent) = state.parent_registry_url.as_deref() else { return Ok(Json(own)); };
    if hop.hops >= curated_list::MAX_LIST_HOPS {
        debug!(list = E::LIST_KEY, hops = hop.hops, "hop limit reached; not forwarding");
        return Ok(Json(own));
    }
    match curated_list::fetch::<E>(&state.client, parent, &filter, hop.hops + 1).await {
        Ok(list) => Ok(Json(list)),
        Err(e) => {
            warn!(list = E::LIST_KEY, parent, error = %e, "parent registry did not answer the list");
            Err((StatusCode::BAD_GATEWAY, format!("parent registry did not answer: {e}")))
        }
    }
}
```

`ListState` holds a `reqwest::Client`, which is `Debug`; derive `Debug` on
`ListState` only if a lint asks for it.

### Step 4 — `crates/community_registry/src/registry.rs`: wire it in

1. Add a field to `EcosystemRegistry` (~l.37): `curated_lists: Router`. Add
   it to the hand-written `Debug` impl (~l.45): `.field("curated_lists", &self.curated_lists)`
   (`axum::Router` is `Debug`).
2. `init` (~l.81): take the role once, then build the lists. Startup fails
   here on a bad entry (AC-13), because `RuntimeServices` calls `init` with
   `?` (`crates/substrate/src/runtime/services.rs:111`).

   ```rust
   let role = config.roles.community_registry.as_ref().ok_or_else(|| {
       anyhow::anyhow!("community registry role must be enabled to initialize registry")
   })?;
   let curated_lists = curated_list::router(role)?;
   // bind_address = role.http_bind_address.clone();
   // parent_registry_url = role.parent_registry_url.clone();
   ```
3. `spawn` (~l.133): after `.with_state(self.state.clone())`, add
   `.merge(self.curated_lists.clone())`. `with_state` returns `Router<()>`,
   so the merge type-checks.

`registry.rs` grows by about 10 lines (361 now). Keep the list code in
`curated_list.rs`.

### Step 5 — `crates/core` unit tests

File `crates/core/src/curated_list/tests.rs` (`#[cfg(test)] mod tests;` at
the end of `curated_list.rs`). Test names describe behaviour, with no
planning ids.

**Normaliser** (one table test for accepted values, one for rejected):

| Input | Output |
| --- | --- |
| `HTTPS://Relay.Example.ORG:443/` | `https://relay.example.org` |
| `https://relay.example.org/` | `https://relay.example.org` |
| `https://relay.example.org` | `https://relay.example.org` |
| `http://relay.example.org:80` | `http://relay.example.org` |
| `https://relay.example.org:8443/` | `https://relay.example.org:8443` |
| `https://relay.example.org/base/` | `https://relay.example.org/base` |
| `http://127.0.0.1:7964` | `http://127.0.0.1:7964` |

Rejected: `relay.example.org`, `localhost:7964`, `ftp://relay.example.org`,
`https://user@relay.example.org`, `https://relay.example.org/?a=1`,
`https://relay.example.org/#x`, `""`.

*Check while you work:* `localhost:7964` parses as a URL with scheme
`localhost` in `url::Url`; it must be rejected by the scheme rule. Confirm the
table with the real `Url` before you rely on it.

**Rank key — fixed vector (AC-4).** These values were computed outside Rust
(Python `hashlib`), so the test does not check the code against itself:

```text
chooser = did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK
rank_key(chooser, "https://relay-a.example.org") = e237c8f6954d7984e41b684dbdcfc7db057a283d10c920abfb4310523518ad6c
rank_key(chooser, "https://relay-b.example.org") = d5223d837045ea1f3d422235d43cd6e91491b1cd94eb83381bee8a821b311ced
rank_key(chooser, "https://relay-c.example.org") = 74abc4c2c13ab5078cfef0953022888b1ef9c32c018d940fa1d0aaaeb92867bf
order: c, b, a

chooser2 = did:key:z6MkfQzd5zjJXbQMG8ZcpvJJT7XKsojbdj7TYV2vpjQ6p1kj
rank_key(chooser2, "https://relay-a.example.org") = b9420b65fca19fef891c85c51478948e8fe8a2f271e4f16b54bc8bc909a5e7e6
rank_key(chooser2, "https://relay-b.example.org") = f1bf8d371f74879c13883e56a31f99bc63ea521640a3c9472a18ae33ad3e2dc5
rank_key(chooser2, "https://relay-c.example.org") = ffdb20eb1ec5ec4ed0d2c887b380123cfda340403d20bcd4307db388abb2affc
order: a, b, c
```

To check the values again:

```bash
python3 -I -c 'import hashlib,struct,sys
a,b=(s.encode() for s in sys.argv[1:3])
print(hashlib.sha256(struct.pack(">I",len(a))+a+struct.pack(">I",len(b))+b).hexdigest())' \
  did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK https://relay-a.example.org
```

Tests:
- `rank_key` equals the three hex values above (compare with `hex::encode`).
- `rank` gives order c, b, a for `chooser`, and a, b, c for `chooser2`.
- `rank` normalises before it keys: an entry with `https://Relay-A.example.org:443/`
  ranks in the same place as `https://relay-a.example.org`, and comes back
  normalised.
- `rank` drops a bad entry and keeps one of two entries with the same
  normalised URL.
- Removing an entry that was not first keeps the first entry the same
  (remove `a` from the `chooser` list: `c` is still first).

**`check_entries` (AC-13):**
- normalises `url` and `info_url` in place;
- a bad `url` in entry 1 fails, and the error text contains
  `roles.community_registry.coordinators[1]` and `url`;
- a bad `info_url` fails and names `info_url`;
- `https://relay.example.org` and `HTTPS://relay.example.org:443/` fail as a
  duplicate, and the error names `[1]` and `[0]`.

**`CuratedList` serde:**
- serialising one entry gives exactly the JSON of the change doc example
  (compare `serde_json::Value`s);
- an unknown top-level key and an unknown entry field are ignored;
- a missing `max_age_secs` or `coordinators` key is an error.

**`fetch`** is tested through the registry in step 6.

**Config** (`crates/core/src/config/tests.rs`, 154 lines now):
- a TOML with two `[[roles.community_registry.coordinators]]` tables parses;
  `list_max_age_secs` defaults to 86400 and `coordinators` to empty when the
  keys are absent;
- `transport = "webrtc"` fails to parse.

### Step 6 — `crates/community_registry` tests

File `crates/community_registry/src/curated_list/tests.rs` (`#[cfg(test)]
mod tests;` at the end of `curated_list.rs`). Do not add to
`registry/tests.rs` (651 lines). Bind registries on `127.0.0.1:0`, like the
existing `spawn_registry_with_parent` (`registry/tests.rs:45`); this is not a
hardcoded port.

Helper:

```rust
async fn spawn_list_registry(
    coordinators: Vec<CoordinatorEntry>,
    parent_registry_url: Option<String>,
) -> anyhow::Result<(EcosystemRegistry, String)>
// SubstrateConfig with roles.community_registry = ServiceRegistryRole {
//   http_bind_address: "127.0.0.1:0", coordinators, parent_registry_url,
//   ..Default::default() }; init, bind, spawn; return (registry, url).
```

and a `get_coordinators(url) -> reqwest::Response` that calls
`{url}/coordinators?transport=iroh-relay`. Call the shared `fetch` where a
parsed list is wanted, to test it too.

Tests:
1. **Serves its list (AC-3).** Two entries; `fetch::<CoordinatorEntry>`
   returns both, normalised, with `max_age_secs = 86400`. A second registry
   with `list_max_age_secs = 60` answers `60`.
2. **Bad `transport` is 400.** No `transport` → 400. `transport=webrtc` → 400.
3. **Bad config stops startup (AC-13).** `EcosystemRegistry::init` returns
   `Err` for a bad `url`, and for two URLs equal after normalisation. The
   error text names the entry.
4. **Empty list asks the parent (AC-15).** Parent P has one entry. Child C
   has none and `parent_registry_url = P`. `GET C` returns P's entry and P's
   `max_age_secs`.
5. **Own list wins.** C has an entry and a parent URL that refuses
   connections. `GET C` returns `200` with C's entry (no forward happened).
6. **Hop limit (AC-15).** Chain R0 → R1 → R2 → R3 → R4 (each child's parent
   is the next). Only R4 has an entry: `GET R0` returns an empty list with
   `200` (R3 gets `hops=3` and does not forward). Only R3 has an entry:
   `GET R0` returns R3's entry. Spawn the chain from the end (R4 first) so
   each parent URL is known.
7. **Loop stops.** Two registries that name each other as parent cannot be
   built in one pass (each needs the other's URL). Instead, call R0 with
   `hops=3` directly: an empty R0 with a parent answers `200` empty and does
   not call the parent (give it a parent URL that refuses connections; a
   `200` proves no forward).
8. **Parent down is 502 (D-A2-3).** C has no entries and a parent URL that
   refuses connections. `GET C` → `502`.
9. **No parent, empty list.** → `200`, empty `coordinators`.

A URL that refuses connections: bind a `TcpListener` on `127.0.0.1:0`, read
its address, drop the listener, use `http://<addr>`.

**Done-rule check.** In the same test file, define a test-only second entry
type and prove the generic route works with only the trait impl:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TestEntry { id: String }
#[derive(Debug, Deserialize)]
struct NoFilter {}
impl ListEntry for TestEntry {
    const LIST_KEY: &'static str = "test-entries";
    type Filter = NoFilter;
    fn matches(&self, _: &NoFilter) -> bool { true }
    fn filter_pairs(_: &NoFilter) -> Vec<(&'static str, String)> { Vec::new() }
    fn check(&mut self) -> anyhow::Result<String> {
        anyhow::ensure!(!self.id.is_empty(), "empty id");
        Ok(self.id.clone())
    }
}
```

Mount `list_route::<TestEntry>(...)` on a bare `axum::Router`, serve it on
`127.0.0.1:0`, and check `GET /test-entries` returns
`{ "max_age_secs": ..., "test-entries": [...] }`, and that a duplicate id
fails `list_route`. `NoFilter {}` is the shape the mailbox list (no filter)
will use.

### Step 7 — imports, lints, sizes

- Import clean-up pass over every edited file (AGENTS.md rule).
- `cargo clippy -q --workspace --all-targets --all-features -- -D warnings`.
- No function over 100 lines; no match arm over ~10 lines.
- `cargo dupes report` over the changed files (recipe in AGENTS.md). The
  hand-written `Deserialize` visitor may look like the one in
  `crates/core/src/config/access.rs`; they parse different shapes, so that is
  fine, but check the duplication percent and lower the ratchet if it falls.

## 4. Things that are ambiguous or stale in task.md and the change docs

For A2:

1. **No config key for `max_age_secs`.** Decided in D-A2-1.
2. **"Forwarding stops after 3 hops" does not say what the last registry
   answers, or what a failed forward answers.** Decided in D-A2-2 and D-A2-3.
3. **The URL form lists three rules only.** Paths, queries, user info and
   fragments are not covered. Decided in D-A2-5. `info_url` is not covered.
   Decided in D-A2-6.
4. **The normalised URL has no trailing slash, but A1 publishes `RelayUrl`
   form with a trailing slash** (change doc D3). A4 must compare a saved
   choice and a list entry through `normalize_url`, never as raw strings,
   and must parse the chosen URL into `RelayUrl` before `HomeRelay::set`.
   Write this into the change doc as a note for A4.
5. **An empty list is cached like a full one.** Decided in D-A2-10; A4 must
   handle it.
6. **"Done when adding a second list needs only a config field, an entry type
   and one route."** The mailbox list (C7) has no `transport`. The generic
   `Filter` type, with `NoFilter {}`, covers it; the test in step 6 proves it.
   The mailbox ranking chooser is the service id, not a substrate DID; `rank`
   takes any chooser string, so it fits.
7. **task.md's change-doc Tasks row** "Config: make `url` optional, …, add the
   registry `coordinators` list and its checks …" mixes A1 and A2 work. A1 did
   the first half and left it unticked. Tick it when this slice is done.

Stale facts in task.md §1 (not used by A2; fix them in the same docs commit
so the next slice does not trust them):

- "The relay URL in the published record is read once … `router.rs:218-222`,
  `set_republish_trigger`": after A1 the publisher reads a `watch` channel and
  wakes on `relay_rx.changed()` (change doc D11,
  `crates/substrate/src/runtime/publish.rs:100`).
- "any unknown `access` string falls through to `Everyone`
  (`crates/coordinator_iroh/src/config.rs:82-98`)": A1 made the parse strict
  (`crates/core/src/config/access.rs`).
- "`crates/community_registry/src/registry.rs:243-281`" (the refresh rule):
  `admit_endpoint` now starts at l.258 (its doc comment at about l.240). Recheck before citing.

## 5. Completion steps

The implementer must follow these steps.

1. **Verify.** Run `mise run verify` as a background job (about 20 minutes)
   and read its summary. Every gate must pass. No `--skip` in the final run.
   While you iterate:
   `cargo nextest run -p syneroym-core curated_list`,
   `cargo nextest run -p syneroym-community-registry`,
   `cargo nextest run -p syneroym-coordinator-iroh --test multi_hop_relay`.
2. **Change docs.**
   - `docs/planning/changes/registry-listed-coordinators/change.md`: fill the
     Test column of AC-3, AC-4, AC-13 and AC-15 with the test files and
     names; tick the Tasks "Config: …" (see §4 item 7) and "Registry:
     `GET /coordinators` and parent forwarding with a hop limit"; add to
     `## Deviations` D-A2-1 to D-A2-10 in one line each; add the A4 notes
     from §4 items 4 and 5; add any change you made to this plan. Add to
     `living-docs-touched` every living doc you change. Keep
     `status: in-progress`.
   - `docs/planning/changes/service-mailbox/change.md`: one sentence where it
     describes `GET /mailboxes`: the list uses the shared
     `list_max_age_secs` setting and the same forwarding and `502` rule.
3. **Living docs.**
   - `docs/system-architecture.md`:
     - config table (~l.373): add rows
       `[roles.community_registry] list_max_age_secs` (default `86400`) and
       `[[roles.community_registry.coordinators]]` (`transport`, `url`,
       `info_url`, `operator`; checked at startup; list only relays with
       `access = "everyone"`);
     - the registry section (~l.2100, next to the parent paragraph): a short
       paragraph on curated lists: `GET /coordinators?transport=iroh-relay`,
       the answer shape, `max_age_secs`, forwarding to the parent when the
       list is empty, at most 3 forwards, `502` when the parent fails, the
       URL normal form, and the rank key. Say that no substrate uses the list
       yet (A4 adds that); do not describe A4's resolver as built.
   - `docs/developer-guide.md` (~l.316, next to the `curl …/lookup/…`
     example): add
     `curl 'http://localhost:7961/coordinators?transport=iroh-relay'`.
   - `docs/TERMINOLOGY.md`: add "Curated list" (a hand-written list in the
     registry config that the registry serves; the coordinator list is one).
   - `docs/planning/traceability-matrix.md`: row `[TOP-REG]` (~l.25): add
     that the registry serves curated lists, with a link to
     `crates/community_registry/src/curated_list.rs`.
   - `docs/system-requirements-spec.md`: no change expected (the
     user-visible relay choice comes in A4). Check once.
4. **Status and backlog.**
   - `status.md`: add an A2 row (Done, the tool that implemented it, the
     `mise run verify` result and the test files).
   - `docs/planning/deferred-backlog.md`: add one row "Curated-list
     forwarding has no cache and no body size limit" (D-A2-4: each request to
     an empty registry costs one parent request; the parent's body is read
     without a size cap). Target `TBD`. Link
     `crates/community_registry/src/curated_list.rs`. Add a row for any
     `TODO`/`FIXME` you leave.
5. **Stop.** Do not start A3 or A4. If you are not making progress after a
   reasonable number of attempts, stop and report what blocks you, with the
   failing output.
