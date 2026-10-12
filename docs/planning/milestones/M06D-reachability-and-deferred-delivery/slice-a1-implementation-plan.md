# M06D slice A1 — Relay plumbing: implementation plan

*Reader: the developer (or AI) who implements slice A1. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  comments (no "M06D", "A1", "AC-10", "H-6", "D2" in code, comments or test
  names; ADR ids are fine), `common::alloc_ports` and the shared harness in
  substrate tests, simple English in comments and docs.
- `docs/planning/milestones/M06D-reachability-and-deferred-delivery/task.md`:
  §1 (row X1), §2 (decision 4, the `access` shape), §3 (slice A1).
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-1, AC-9,
  AC-10, AC-11, AC-14, AC-16, and the design parts "Substrate side",
  "Home-relay handle", "Degraded mode", "Why not `RelayMode::Disabled`",
  "Removing the n0 fallback", and `## Deviations`.
- `docs/planning/changes/operator-passport/change.md`: H-6 and "Access
  setting".
- `crates/coordinator_iroh/tests/relay_map_runtime.rs`: the iroh 0.97
  behaviour this slice builds on.

**Branch.** Create `feat/m06d-a1-relay-plumbing` from the latest `main`:

```bash
git fetch origin main
git checkout -b feat/m06d-a1-relay-plumbing origin/main
```

Commit on this branch and push it with `git push -u origin
feat/m06d-a1-relay-plumbing`. Do not open a pull request unless the user asks.
Commit messages follow the Conventional Commits and 50/72 rules in
`AGENTS.md`.

### Terms used in this plan

- **Home relay.** The one Iroh relay that an endpoint stays connected to. Peers
  reach the endpoint through it. Its URL goes into the endpoint's signed
  record.
- **Confirmed (home relay).** iroh has chosen the relay as its home relay, so
  `Endpoint::addr()` lists it. iroh chooses it when a net-report probe reaches
  the relay. This happens *before* the relay connection opens and before the
  relay runs its access check (`iroh-0.97.0/src/socket/transports/relay/actor.rs:966-985`).
  So "confirmed" does not prove the relay admits this node.
- **Degraded mode.** The substrate has an Iroh endpoint with an empty relay
  map (`RelayMode::Custom(RelayMap::empty())`). It can dial a peer through that
  peer's relay (spike test
  `endpoint_without_home_relay_dials_peer_through_peer_relay`: the dialing
  endpoint has no relay of its own). Its local services, its client gateway and
  its own coordinator role do not use its Iroh endpoint, so they keep working.
  Other nodes cannot reach it through a relay, and it publishes no endpoint
  record of its own (D4). It leaves degraded mode when a relay is set through
  the home-relay handle.
- **The n0 preset.** `iroh::endpoint::presets::N0`: the public relays and DNS
  address lookup of n0 (the maker of Iroh). This slice removes every runtime
  use of it.
- **QAD.** QUIC address discovery: a relay tells a node its public address.
  `RelayConfig::from(url)` turns it on and probes UDP port 7842
  (`iroh-relay-0.97.0/src/defaults.rs:7`).

### Facts about iroh 0.97 that the design must respect

These come from the spike (`relay_map_runtime.rs`) and from reading
`iroh-0.97.0/src/endpoint.rs`, `socket.rs` and `net_report`.

1. With an empty relay map, `Endpoint::online()` never finishes. So boot must
   never wait for `online()`. Wait for the home relay in the background.
2. After `remove_relay` of the *last* relay, `addr().relay_urls()` still lists
   it, because iroh skips net reports while the map is empty
   (`socket.rs:694`). So never read the record's relay URL from iroh. Our own
   `watch` channel is the source. Publish a relay URL only after iroh confirms
   it, with a timeout.
3. With two relays in the map, iroh keeps one home relay (in the spike, the
   old one). The switch order is: insert the new relay, then remove the old
   one, so the map is never empty during a switch. In normal running the map
   holds exactly one relay.
4. `insert_relay` and `remove_relay` both send `RelayMapChange`, which starts a
   new net report (`socket.rs:1113-1133`, `1418`). A report that is still
   running makes the next one wait (`socket.rs:653`, `want_update`). So a
   switch costs two reports in a row. iroh also runs a report on its own every
   20 to 26 s (`socket.rs:1616-1630`).
5. When a report finds no relay that answers, iroh keeps its current home
   relay and stays connected to it (`socket.rs:1598-1605`). It chooses only
   among relays in the current map.
6. The time to confirm: the spike measured about 300 ms with `quic: None` (no
   QAD). This plan keeps `RelayConfig::from(url)` (QAD on, as today). Our
   relays do not serve QAD on 7842, so each QAD probe runs until
   `PROBES_TIMEOUT` (3 s), and one report can take up to 5 s
   (`net_report/defaults.rs:17,23`). Two reports for a switch can take about
   10 s. So the confirm timeout is 15 s (D10). This is read from the code,
   not measured: test 13a logs the real times, and the implementer records
   them in `## Deviations`.
7. The watcher from `Endpoint::watch_addr()` disconnects only when the *last*
   `Endpoint` clone is dropped. Closing the endpoint does not disconnect it
   (`endpoint.rs:1104-1106`). `HomeRelay` holds a clone, so a wait on the
   watcher must race against `Endpoint::closed()` to end on shutdown.
8. `iroh::Watcher` is re-exported (`n0_watcher::Watcher`).
   `Endpoint::watch_addr()` returns a watcher with `get()` and `updated()`.
9. `RelayUrl::from_str` is only `Url::from_str`
   (`iroh-base-0.97.0/src/relay_url.rs:38-44`). `"localhost:7964"` parses
   with the scheme `localhost`, and `"ftp://x"` parses too. A real check needs
   the scheme and the host (D2).

## 1. What this slice delivers

- **Home-relay handle** (`HomeRelay`, new, in `crates/router`). The router
  builds the substrate's Iroh endpoint with an empty relay map, and returns a
  handle that sets the home relay at run time. The handle keeps the confirmed
  home relay URL in a `tokio::sync::watch` channel, and keeps track of the
  relay that is really in the map. A switch that is not confirmed rolls back.
- **Home-relay source** (`HomeRelaySource`, new, in `crates/substrate`). In
  this slice it has one variant, `Fixed { url }`, built from
  `parent_coordinator.iroh.url`. A background task sets it through the handle
  and keeps waiting until iroh confirms it. Boot never waits for it.
- **Record publisher reads the watch channel.** It never publishes the
  substrate's own record without a relay URL (D4). Its loop wakes up when the
  home relay changes, so the record appears as soon as a relay is confirmed.
- **No n0 relays anywhere at run time.** `net_iroh::build_iroh_endpoint`
  always uses `RelayMode::Custom(..)`: one relay, or an empty map when there is
  no URL.
- **`parent_coordinator.iroh.url` is optional.** No URL and `iroh` in
  `communication_interfaces` means degraded mode, with a clear warning. A URL
  that is not a valid `http`/`https` URL with a host stops startup with a clear
  error (D2). The dev profile writes its `localhost` relay explicitly.
- **`coordinator_discovery_url` is removed.**
- **Strict `[roles.coordinator] access`.** A new config type in its final shape.
  `"everyone"` and a list of endpoint ids work. `"passport"`, the table form
  and any unknown value stop the coordinator at startup with a clear error.

Out of scope: the registry list, ranking, `coordinators.json`, failover, the
registry retry (A2 to A4); `/v1/info` changes beyond what step 8 lists (A3);
anything about passports beyond parsing. Do not start A2.

## 2. Design decisions made in this plan

Record each of these in the coordinators change doc `## Deviations` section if
it differs from the change doc's Design or criteria (see Completion steps).

- **D1. `ParentCoordinatorConfig.iroh` is no longer an `Option`.** It becomes
  `IrohParentConfig { url: Option<String> }`, with `#[serde(default)]`. A
  missing `[parent_coordinator.iroh]` section and a section without `url` mean
  the same thing: no fixed relay. To run without Iroh, remove `iroh` from
  `communication_interfaces`.
- **D2. A bad `url` stops startup; a missing `url` is degraded mode.**
  - A `parent_coordinator.iroh.url` that is not valid is an operator mistake.
    Nothing at run time can fix it, so a degraded node would only hide it.
    "Valid" means: it parses as a URL, the scheme is `http` or `https`, and it
    has a host (fact 9). The substrate and both coordinator roles stop at
    startup with an error that names the setting and the bad value. Each
    coordinator checks it first in `init`, before it binds any listener.
  - This changes the AC-10 text, which said "empty relay map" for this case.
    AC-10's goal (never the n0 preset) still holds. Record it in
    `## Deviations`.
  - A missing `url` is different. The node is still useful: its local
    services, its gateway and its outbound calls through peers' relays all
    work. A relay can also arrive later: through the home-relay handle in
    this slice, through `roymctl substrate home-relay set` (slice B3), and
    through the registry retry (slice A4). So the node boots, logs a clear
    warning, and waits.
  - Known gap until A4 and B3 land: nothing outside tests calls the handle. A
    degraded node stays degraded until it restarts with a `url`. The warning
    says so. Add this to the deferred backlog.
  - Parsing happens once, in one helper (`net_iroh::parent_relay_url`).
    `build_iroh_endpoint` takes an already-parsed `Option<RelayUrl>`, so it
    has no "bad URL" path at all.
- **D3. Relay URLs are written in `RelayUrl` form.** `RelayUrl` adds a
  trailing slash (`http://127.0.0.1:7964` becomes `http://127.0.0.1:7964/`).
  This reaches three places: the substrate record, the `/v1/info`
  `relay_url` field, and the Iroh coordinator's own registry record. Every
  reader parses the field as `RelayUrl` (`sdk/src/client.rs:375-380`,
  `net_iroh.rs` `resolve_iroh_addr`, `multi_hop_relay.rs:361`,
  `connection_limit.rs:212`, `smoke-tests/src/main.rs:223`), so this is
  harmless. No test compares the string.
- **D4. No substrate record without a relay URL.** The record holds no direct
  addresses (`build_signed_endpoint_info` prunes them). So a record without a
  relay URL cannot be dialed, and publishing it only puts a useless record in
  front of callers.
  - Each heartbeat pass skips the substrate's own record while the confirmed
    home relay is `None`. It still replays the hosted service records, as
    today.
  - The first confirmed relay wakes the publish loop, so the record appears
    within about a second of the confirmation.
  - The same rule covers both cases: the first boot with a fixed relay that is
    still down, and degraded mode. No timed wait is needed.
  - This also keeps today's behaviour for configs with no
    `[parent_coordinator.iroh]` section: they had no Iroh endpoint, so they
    published no substrate record.
- **D5. A switch that is not confirmed rolls back.** iroh keeps its old home
  relay when the new one does not answer (fact 5), so the old relay still
  works. On timeout, `set` inserts the old relay again, removes the new one,
  and keeps the watch channel on the old relay. The record stays correct and
  keeps being refreshed. For a *first* `set` (no old relay), the new relay
  stays in the map so iroh keeps trying it, and the watch channel stays
  `None`.
  - Known limit: the rollback itself is not confirmed. If the new relay is
    confirmed just after the timeout, and the old relay does not answer in
    the rollback's net report, iroh's home relay is the new relay (no longer
    in the map) while the watch names the old one. The window is very small.
    After the rollback, `set` reads `addr()` once and logs a warning if it
    does not list the old relay. Record this limit in `## Deviations`.
- **D6. The Iroh coordinator does not wait for `online()` at boot either.** The
  only reader that needed it was the `/v1/info` payload, which read the relay
  address from `iroh_endpoint.addr()`. It now uses the relay URL the
  coordinator already chose. A background task logs when the endpoint comes
  online. The coordinator keeps a fixed relay map built at boot; it does not
  use `HomeRelay` (the handle is for the substrate's own endpoint). Existing
  tests that dial a coordinator right after `init` wait for its relay through
  a small test helper (step 13d).
- **D7. New type `CoordinatorAccess` for `[roles.coordinator] access` only.**
  `[roles.community_registry] access` keeps the old `AccessControl` type. No
  code reads that field (checked with grep), so it is out of scope. Add it to
  the deferred backlog.
- **D8. In the table form, `passport` must be `true`.** `{ list = [...],
  passport = false }` is a parse error that says "use the plain list form".
  One meaning per shape. Unknown keys in the table are an error.
- **D9. Unknown `access` strings fail at config parse time.** The error names
  the bad value and the accepted forms. The whole process stops at startup,
  which includes the coordinator. `"passport"` and the table form parse, and
  `CoordinatorIroh::init` rejects them, so slice B2 only changes that check,
  not the type.
- **D10. The confirm timeout is 15 s, and QAD stays on.** A switch needs two
  net reports, and with QAD probes to an unserved port each can take up to
  5 s (facts 4 and 6). Turning QAD off (`quic: None`) would make reports fast,
  but it would also turn it off for any relay that does serve QAD on 7842.
  That is a policy change for slice A4, when third-party relays come from the
  registry. Add a backlog row for the QAD port mismatch.
- **D11. The publish loop wakes on relay changes directly.** The change doc
  says the record is published again "through the existing republish trigger".
  This plan adds a `relay_rx.changed()` arm to the same `select!` that already
  serves that trigger, instead of a bridge task that pushes requests into the
  trigger channel. Same loop, same publish code, one task fewer, and the
  `republish_now` channel is not shared with relay events. Record it in
  `## Deviations`.

## 3. Step-by-step changes

Do the steps in this order. Each step names the file, the change and the
callers. After steps 1 to 3, the workspace does not compile until the callers
are fixed; use the compiler errors as the list of remaining callers.

### Step 1 — `crates/core`: optional relay URL, remove `coordinator_discovery_url`

File `crates/core/src/config/base.rs`:

1. Delete `fn default_relay_url()`.
2. Replace `IrohParentConfig` and its `impl Default` with:

   ```rust
   /// `[parent_coordinator.iroh]`: the home relay of this substrate's Iroh
   /// endpoint.
   #[derive(Debug, Clone, Default, Serialize, Deserialize)]
   #[serde(default)]
   pub struct IrohParentConfig {
       /// The relay URL. When it is not set, the endpoint starts with an
       /// empty relay map: it can dial peers through their relays, but no
       /// peer can reach it through a relay.
       pub url: Option<String>,
   }
   ```

3. In `ParentCoordinatorConfig`, change `pub iroh: Option<IrohParentConfig>`
   to `pub iroh: IrohParentConfig`.
4. In `SubstrateGlobalConfig`, delete the field `coordinator_discovery_url`
   and its line in `impl Default`. Old config files that still have the key
   keep parsing: `SubstrateGlobalConfig` has `#[serde(default)]` and no
   `deny_unknown_fields`, so serde ignores it.

Tests, in `crates/core/src/config/tests.rs` (add `toml.workspace = true` under
`[dev-dependencies]` in `crates/core/Cargo.toml`):

- `iroh_relay_url_is_none_without_the_section`:
  `toml::from_str::<SubstrateConfig>("")` gives
  `parent_coordinator.iroh.url == None`.
- `iroh_relay_url_is_none_in_an_empty_section`: `"[parent_coordinator.iroh]\n"`
  gives `None`.
- `iroh_relay_url_is_read_from_the_section`: `url = "http://relay.example:7964"`
  gives `Some(..)`.
- `old_coordinator_discovery_url_key_is_ignored`: a config with
  `[substrate]\ncoordinator_discovery_url = "http://x"` still parses.

### Step 2 — `crates/core`: the `CoordinatorAccess` type

New file `crates/core/src/config/access.rs`, with its tests in
`crates/core/src/config/access/tests.rs` (sibling-file layout; declare
`#[cfg(test)] mod tests;` at the end of `access.rs`). In
`crates/core/src/config.rs` add `pub mod access;` next to the other modules
and `pub use access::*;` next to the other re-exports.

Type:

```rust
/// Who may use the Iroh relay of a coordinator: `[roles.coordinator] access`.
///
/// Four forms parse. A coordinator may refuse to start with a form it cannot
/// enforce yet; parsing never widens a value to "everyone".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CoordinatorAccess {
    /// `access = "everyone"`: any Iroh endpoint. The default.
    #[default]
    Everyone,
    /// `access = "passport"`: only endpoints with a running passport lease.
    Passport,
    /// `access = ["<endpoint id>", ...]`: only the listed endpoints.
    List(Vec<String>),
    /// `access = { list = ["<endpoint id>", ...], passport = true }`: a listed
    /// endpoint, or one with a running passport lease.
    ListOrPassport(Vec<String>),
}
```

Keep endpoint ids as `String`. `syneroym-core` does not depend on iroh; the
coordinator parses them (step 9). (`PartialEq` is used by the tests below.)

**`Deserialize`: write it by hand with a `Visitor`**, so that every error
names the accepted forms. A derived `#[serde(untagged)]` enum gives only "data
did not match any variant", which is not a clear error. Import
`serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor}` and
`serde::de::value::MapAccessDeserializer`; no inline paths with several `::`.

```rust
const EXPECTED: &str =
    "\"everyone\", \"passport\", a list of endpoint ids, or { list = [...], passport = true }";

impl<'de> Deserialize<'de> for CoordinatorAccess {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(AccessVisitor)
    }
}

struct AccessVisitor;

impl<'de> Visitor<'de> for AccessVisitor {
    type Value = CoordinatorAccess;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(EXPECTED)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        match value {
            "everyone" => Ok(CoordinatorAccess::Everyone),
            "passport" => Ok(CoordinatorAccess::Passport),
            other => Err(E::custom(format!(
                "unknown [roles.coordinator] access value {other:?}; expected {EXPECTED}"
            ))),
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut ids = Vec::new();
        while let Some(id) = seq.next_element::<String>()? {
            ids.push(id);
        }
        Ok(CoordinatorAccess::List(ids))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let table = AccessTable::deserialize(MapAccessDeserializer::new(map))?;
        if !table.passport {
            return Err(de::Error::custom(
                "[roles.coordinator] access table with `passport = false`: write the \
                 plain list form `access = [...]` instead",
            ));
        }
        Ok(CoordinatorAccess::ListOrPassport(table.list))
    }
}

/// The table form, `{ list = [...], passport = true }`. Both keys are required.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessTable {
    list: Vec<String>,
    passport: bool,
}
```

Any other input type (a number, a boolean) falls to serde's default
"invalid type ... expected <EXPECTED>" message, which already names the forms
through `expecting`.

**`Serialize`: write it by hand** so it writes back the same four shapes:
`Everyone` → string `"everyone"`, `Passport` → `"passport"`, `List(ids)` →
the sequence, `ListOrPassport(ids)` → a struct `{ list, passport: true }`
(a small private `#[derive(Serialize)] struct AccessTableRef<'a> { list: &'a
[String], passport: bool }`).

In `crates/core/src/config/roles.rs`, change `CoordinatorRole.access` from
`AccessControl` to `CoordinatorAccess`. Leave `ServiceRegistryRole.access`
and the `AccessControl` type as they are (D7). Update the doc comment above
`AccessControl` to say it is the registry's setting.

Tests in `crates/core/src/config/access/tests.rs`. Parse a small wrapper,
`#[derive(Deserialize, Serialize)] struct Wrap { access: CoordinatorAccess }`,
with `toml::from_str`:

- each accepted form: `"everyone"`, `"passport"`, `["a", "b"]`, `[]`,
  `{ list = ["a"], passport = true }`, each to the expected variant;
- missing key: `toml::from_str::<CoordinatorRole>("")` gives `Everyone`;
- a typo, `"everyon"`: error text contains `"everyon"` and `everyone`;
- another unknown string, `"anyone"`: error;
- `{ list = ["a"], passport = false }`: error that mentions the list form;
- `{ list = ["a"] }` (missing `passport`) and `{ passport = true }` (missing
  `list`): error;
- `{ lists = ["a"], passport = true }` (unknown key): error;
- `access = 1`: error;
- serialize round trip for all four variants (`toml::to_string` of `Wrap`,
  then parse back, equal).

### Step 3 — `crates/router`: relay URL parsing and no n0 preset

File `crates/router/src/net_iroh.rs`:

1. Remove `presets::N0` from the `iroh::endpoint` import.
2. Add the parsing helpers. They are the only place that turns a config
   string into a `RelayUrl`:

   ```rust
   /// Parses a relay URL from a config setting. A relay URL must be `http`
   /// or `https` and have a host. The error names the setting and the value,
   /// because a bad value stops startup.
   ///
   /// # Errors
   /// When `raw` is not such a URL.
   pub fn parse_relay_url(setting: &str, raw: &str) -> anyhow::Result<RelayUrl> {
       let url = raw
           .parse::<RelayUrl>()
           .with_context(|| format!("{setting} = {raw:?} is not a valid URL"))?;
       // `RelayUrl` derefs to `url::Url`.
       anyhow::ensure!(
           matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
           "{setting} = {raw:?} must be an http:// or https:// URL with a host"
       );
       Ok(url)
   }

   /// The parent relay of this node: `parent_coordinator.iroh.url`, parsed.
   /// `Ok(None)` when it is not set.
   ///
   /// # Errors
   /// When it is set and not a valid relay URL.
   pub fn parent_relay_url(config: &IrohParentConfig) -> anyhow::Result<Option<RelayUrl>> {
       config
           .url
           .as_deref()
           .map(|raw| parse_relay_url("parent_coordinator.iroh.url", raw))
           .transpose()
   }
   ```

   `syneroym-router` has no direct `url` dependency, and none is needed:
   `RelayUrl` implements `Deref<Target = Url>` (`iroh-base-0.97.0/src/relay_url.rs:59`).

3. Add a private helper and use it in `build_iroh_endpoint`:

   ```rust
   /// Only the given relay, or an empty map. Never the n0 preset: an endpoint
   /// with an empty map can still dial a peer through that peer's relay, and
   /// can get a relay later with `Endpoint::insert_relay`.
   /// `RelayMode::Disabled` would build no relay transport at all.
   fn relay_mode(relay_url: Option<RelayUrl>) -> RelayMode {
       RelayMode::Custom(relay_url.map_or_else(RelayMap::empty, RelayMap::from))
   }

   pub async fn build_iroh_endpoint(
       relay_url: Option<RelayUrl>,
       secret_key: Option<SecretKey>,
       idle_timeout_secs: Option<u64>,
   ) -> anyhow::Result<Endpoint> {
       let mut builder = Endpoint::empty_builder().relay_mode(relay_mode(relay_url));
       // secret key and idle timeout: unchanged
   }
   ```

   The parameter type changes from `Option<String>` to an already-parsed
   `Option<RelayUrl>`. Callers: three runtime callers and one test.
   - `crates/router/src/connection_router.rs` (step 5): passes `None`.
   - `crates/coordinator_iroh/src/coordinator.rs` (step 8): passes the
     resolved relay URL.
   - `crates/coordinator_webrtc/src/coordinator.rs`: at the very start of
     `init`, before it binds any listener (today the listeners bind at
     ~l.65-70), add
     `let iroh_relay_url = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)?;`.
     Line 71-72 becomes
     `let endpoint = net_iroh::build_iroh_endpoint(iroh_relay_url, None, None).await?;`.
     A coordinator role never resolves its parent relay from anywhere else
     (AC-16).
   - `crates/coordinator_webrtc/src/bootstrap.rs:281` (a test): `None` still
     compiles; no change.

Unit tests at the end of `net_iroh.rs` (`#[cfg(test)] mod tests`; the file
is 163 lines, so an in-file module is fine):

- `relay_mode_without_url_is_an_empty_custom_map`
- `relay_mode_with_url_has_only_that_relay` (`relay_map().len() == 1` and it
  contains the URL)
- `parse_relay_url_accepts_http_and_https`
- `parse_relay_url_rejects_a_value_without_scheme`: `"localhost:7964"` is an
  error whose text contains the setting name and the value.
- `parse_relay_url_rejects_other_schemes`: `"ftp://x"`.
- `parse_relay_url_rejects_garbage`: `"not a url"`.
- `parent_relay_url_is_none_without_url`.

Assert relay modes with `matches!(mode, RelayMode::Custom(_))` and
`mode.relay_map()`.

New test file `crates/router/tests/no_n0_relay_preset.rs`: a source scan in
the style of `crates/substrate/tests/no_ephemeral_port_literals.rs`. It walks
every `.rs` file under `crates/*/src` and `apps/*/src` (workspace root is
`Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")`), skips the
`crates/smoke-tests` directory (a test tool with its own n0 builder, excluded
by AC-10), skips lines whose trimmed start is `//`, and fails if any other
line contains `presets`, `RelayMode::Default`, `RelayMode::Staging`,
`Endpoint::builder(` or `Endpoint::bind(`. (`Endpoint::builder` and
`Endpoint::bind` are the constructors that take a preset; runtime code uses
only `Endpoint::empty_builder()`. Matching the word `presets` also catches
`presets::{N0}`.) The failure message lists `file:line` for each hit. Use a
small recursive `fs::read_dir` walk; do not add a dependency. Start the file
with a `//!` comment that says why: the endpoint must never fall back to the
public n0 relays, which show metadata to a third party.

### Step 4 — `crates/router`: the home-relay handle

Add `thiserror.workspace = true` to `[dependencies]` in
`crates/router/Cargo.toml`.

New file `crates/router/src/home_relay.rs`, with unit tests in
`crates/router/src/home_relay/tests.rs`. In `crates/router/src/lib.rs` add
`mod home_relay;` and
`pub use home_relay::{HOME_RELAY_CONFIRM_TIMEOUT, HomeRelay, HomeRelayError};`.

```rust
//! The home relay of this substrate's Iroh endpoint, set at run time.
//!
//! The endpoint starts with an empty relay map. `HomeRelay::set` puts one
//! relay in the map, waits until iroh confirms it as the home relay, and only
//! then shows the URL to subscribers. The URL is never read back from iroh:
//! after the last relay is removed, iroh 0.97 still lists it in
//! `Endpoint::addr`.
//!
//! "Confirmed" means iroh chose the relay after a net-report probe reached
//! it. iroh does this before the relay connection opens and before the relay
//! checks access, so a confirmed relay may still refuse this node. Do not use
//! confirmation as proof of access.

/// How long `HomeRelay::set` waits for iroh to confirm a relay. A switch
/// needs two net reports, and each can take up to 5 s when the relay does not
/// serve QUIC address discovery on the port iroh probes.
pub const HOME_RELAY_CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum HomeRelayError {
    #[error("relay {url} was not confirmed as the home relay within {timeout:?}")]
    NotConfirmed { url: RelayUrl, timeout: Duration },
    #[error("the Iroh endpoint is closed")]
    EndpointClosed,
}

/// A cheap, cloneable handle to the home relay of one endpoint.
#[derive(Debug, Clone)]
pub struct HomeRelay {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    endpoint: Endpoint,
    /// The confirmed home relay. `None` until the first `set` succeeds.
    confirmed: watch::Sender<Option<RelayUrl>>,
    /// The relay that is in the relay map now. The lock also makes switches
    /// run one at a time, so two callers never interleave insert and remove.
    /// It differs from `confirmed` only after a first `set` that was not
    /// confirmed: then the relay stays in the map so iroh keeps trying it.
    in_map: Mutex<Option<RelayUrl>>,
    confirm_timeout: Duration,
}

impl HomeRelay {
    /// `endpoint` must have been built with an empty relay map.
    #[must_use]
    pub fn new(endpoint: Endpoint, confirm_timeout: Duration) -> Self;

    /// The confirmed home relay URL, and every later change of it.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Option<RelayUrl>>;

    /// Makes `url` the home relay.
    ///
    /// # Errors
    /// `NotConfirmed` when iroh does not confirm the relay in time. A switch
    /// from a confirmed relay is then rolled back; a first relay stays in the
    /// map and the caller may call `set` again. `EndpointClosed` when the
    /// endpoint is closed.
    pub async fn set(&self, url: RelayUrl) -> Result<(), HomeRelayError>;
}
```

`Mutex` here is `tokio::sync::Mutex`, imported by name (it is held across
`.await`). There is no `current()`: callers read `subscribe().borrow()`.

`set`, in pseudo-code:

```rust
pub async fn set(&self, url: RelayUrl) -> Result<(), HomeRelayError> {
    let mut in_map = self.inner.in_map.lock().await;
    let endpoint = &self.inner.endpoint;
    if endpoint.is_closed() {
        return Err(HomeRelayError::EndpointClosed);
    }
    let confirmed = self.inner.confirmed.borrow().clone();
    if confirmed.as_ref() == Some(&url) {
        return Ok(());
    }
    // A retry of a first relay that is already in the map only waits again.
    if in_map.as_ref() != Some(&url) {
        replace_relay(endpoint, in_map.as_ref(), &url).await;
        *in_map = Some(url.clone());
    }
    match wait_until_home(endpoint, &url, self.inner.confirm_timeout).await {
        Confirm::Home => {
            self.inner.confirmed.send_replace(Some(url));
            Ok(())
        }
        Confirm::Closed => Err(HomeRelayError::EndpointClosed),
        Confirm::TimedOut => {
            // iroh keeps its old home relay when the new one does not answer,
            // so the old relay still works: put it back.
            if let Some(old) = confirmed {
                replace_relay(endpoint, Some(&url), &old).await;
                if !endpoint.addr().relay_urls().any(|u| *u == old) {
                    // Not confirmed: very unlikely, logged so it can be seen.
                    warn!(relay_url = %old, "rolled back to the old relay, but iroh does not list it as the home relay");
                }
                *in_map = Some(old);
            }
            Err(HomeRelayError::NotConfirmed { url, timeout: self.inner.confirm_timeout })
        }
    }
}

/// Inserts `new`, then removes `old`, so the map is never empty during a
/// switch. With both in the map, iroh keeps the old home relay; removing the
/// old relay starts the net report that picks the new one.
async fn replace_relay(endpoint: &Endpoint, old: Option<&RelayUrl>, new: &RelayUrl) {
    endpoint.insert_relay(new.clone(), Arc::new(RelayConfig::from(new.clone()))).await;
    if let Some(old) = old.filter(|old| *old != new) {
        endpoint.remove_relay(old).await;
    }
}

enum Confirm { Home, Closed, TimedOut }

async fn wait_until_home(endpoint: &Endpoint, url: &RelayUrl, timeout: Duration) -> Confirm {
    let mut addr = endpoint.watch_addr();
    let home = async {
        loop {
            if addr.get().relay_urls().any(|u| u == url) {
                return Confirm::Home;
            }
            if addr.updated().await.is_err() {
                return Confirm::Closed;
            }
        }
    };
    // The watcher does not end when the endpoint closes, only when the last
    // clone is dropped, and this handle holds one.
    let wait = async {
        tokio::select! {
            confirm = home => confirm,
            () = endpoint.closed() => Confirm::Closed,
        }
    };
    time::timeout(timeout, wait).await.unwrap_or(Confirm::TimedOut)
}
```

Invariant: after `set` returns, the map holds exactly one relay, and it is
`in_map`. Import `iroh::Watcher` for `get` and `updated`. If the watcher does
not work as described (for example its future is not `Send` inside a spawned
task), poll `endpoint.addr()` every 50 ms inside the same `select!` instead,
as `wait_for_relays` in the spike does. Write down the change in the change
doc `## Deviations`.

Unit tests in `crates/router/src/home_relay/tests.rs`. They need no relay
server: they use URLs of TCP listeners that never accept, so nothing is ever
confirmed, and they read the private `in_map`.

```rust
/// An `http://127.0.0.1:<port>` URL whose listener accepts TCP but never
/// answers, so iroh can never confirm it. Keep the listener alive for the
/// whole test; never reuse a freed port.
async fn dead_relay() -> (TcpListener, RelayUrl)
```

Bind the test endpoint like the spike's `bind_empty` (empty relay map,
`clear_ip_transports`, loopback `bind_addr` on port 0).

- `first_set_to_a_dead_relay_keeps_it_in_the_map`: `set(dead1)` with a 1 s
  timeout is `NotConfirmed`; `subscribe().borrow()` is `None`; `in_map` is
  `Some(dead1)`.
- `set_after_a_failed_first_set_replaces_the_leftover`: `set(dead1)` fails,
  then `set(dead2)` fails; `in_map` is `Some(dead2)` (dead1 was removed, so the
  map holds one relay).
- `set_on_a_closed_endpoint_is_endpoint_closed`: close the endpoint, then
  `set(dead1)` is `EndpointClosed`.
- `close_during_set_ends_the_wait`: start `set(dead1)` with a 15 s timeout,
  close the endpoint after 200 ms; `set` returns `EndpointClosed` well before
  15 s (assert under 5 s).

### Step 5 — `crates/router`: `ConnectionRouter` builds the endpoint with an empty map

File `crates/router/src/connection_router.rs`:

1. Add a field `home_relay: Option<HomeRelay>` to `ConnectionRouter`, with a
   doc comment: "Sets the home relay of the Iroh endpoint. `None` when `iroh`
   is not in `communication_interfaces`." Initialise it to `None` in `init`.
2. In `init`, the Iroh endpoint is built when `"iroh"` is in
   `config.substrate.communication_interfaces`. Remove the
   `config.parent_coordinator.iroh` condition. Pass `None` as the relay URL:

   ```rust
   let iroh_endpoint = if config.substrate.communication_interfaces.iter().any(|c| c == "iroh") {
       let idle_timeout = /* unchanged */;
       Some(net_iroh::build_iroh_endpoint(None, Some(SecretKey::from_bytes(&iroh_secret_key)), idle_timeout).await?)
   } else {
       None
   };
   router.home_relay =
       iroh_endpoint.clone().map(|ep| HomeRelay::new(ep, HOME_RELAY_CONFIRM_TIMEOUT));
   ```

   Rewrite the comment above it. Keep the reason it is built before
   `RouteHandler::init`. Say that the endpoint starts with no relay, and that
   the substrate sets the home relay in the background through `home_relay()`.
3. `spawn_iroh_router`: delete the `time::timeout(.., ep.online())` block. The
   function then has no `.await`, so make it a plain `fn` returning
   `IrohRouter` (clippy `unused_async`). Keep the `info!` "Iroh listening on
   ALPN" log. Update the call in the `"iroh"` arm of the loop (drop `.await?`).
   Remove the now-unused `Duration` and `time` imports if nothing else uses
   them.
4. Add:

   ```rust
   /// The home-relay handle of the Iroh endpoint, when there is one.
   #[must_use]
   pub fn home_relay(&self) -> Option<HomeRelay> {
       self.home_relay.clone()
   }
   ```

File `crates/router/src/route_handler.rs:370-371`: change to
`let parent_coordinator_url = config.parent_coordinator.iroh.url.clone();`.
No other change (the field `_parent_relay_url` stays as it is).

### Step 6 — `crates/substrate`: the home-relay source

New file `crates/substrate/src/runtime/home_relay.rs`, with unit tests in
`crates/substrate/src/runtime/home_relay/tests.rs`. Add `mod home_relay;` in
`crates/substrate/src/runtime.rs` next to the other `mod` lines.

```rust
//! Where the home relay of this substrate's Iroh endpoint comes from, and the
//! background task that sets it. Boot never waits for this task.

/// The source of the home relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed { url: RelayUrl },
}

impl HomeRelaySource {
    /// The source for this config. `Ok(None)` means degraded mode: there is
    /// no `url`.
    ///
    /// # Errors
    /// When `url` is set and not a valid relay URL. Startup stops.
    pub(super) fn from_config(config: &IrohParentConfig) -> anyhow::Result<Option<Self>> {
        Ok(net_iroh::parent_relay_url(config)?.map(|url| Self::Fixed { url }))
    }
}

/// Starts setting the home relay in the background, or logs the degraded
/// mode warning when there is no source. Never blocks.
pub(super) fn start_home_relay(home_relay: HomeRelay, source: Option<HomeRelaySource>) {
    let Some(source) = source else {
        warn!(
            "parent_coordinator.iroh.url is not set: the Iroh endpoint has no home relay \
             (degraded mode). Local services and outbound calls work, but no peer can \
             reach this node through a relay, and its endpoint record is not published. \
             Set parent_coordinator.iroh.url and restart, or remove \"iroh\" from \
             substrate.communication_interfaces."
        );
        return;
    };
    tokio::spawn(run_source(home_relay, source));
}
```

`run_source` matches on the source:

```rust
async fn run_source(home_relay: HomeRelay, source: HomeRelaySource) {
    match source {
        HomeRelaySource::Fixed { url } => run_fixed(&home_relay, url).await,
    }
}

/// Sets a fixed relay and keeps waiting until iroh confirms it.
///
/// No sleep between tries: after the first try the relay stays in the map,
/// and iroh runs a net report on its own every 20 to 26 s. So when a relay
/// that was down comes back, it is confirmed within about 30 s.
async fn run_fixed(home_relay: &HomeRelay, url: RelayUrl) {
    let mut last_warn: Option<Instant> = None;
    loop {
        match home_relay.set(url.clone()).await {
            Ok(()) => {
                info!(relay_url = %url, "home relay set");
                return;
            }
            Err(HomeRelayError::EndpointClosed) => return,
            Err(e @ HomeRelayError::NotConfirmed { .. }) => {
                // Warn on the first failure, then at most every 5 minutes.
                if last_warn.is_none_or(|t| t.elapsed() >= FIXED_WARN_INTERVAL) {
                    warn!(error = %e, "home relay not confirmed yet; still waiting");
                    last_warn = Some(Instant::now());
                } else {
                    debug!(error = %e, "home relay not confirmed yet; still waiting");
                }
            }
        }
    }
}
```

`set` itself ends on shutdown (`EndpointClosed`), so this loop ends too.

How later slices extend this:

- Slice A4 adds a `Registry` variant and its arm in `run_source`, and
  `from_config` returns it when there is no `url` and a registry is
  configured. Callers do not change.
- Slice B3 adds a registration step at the start of `run_fixed`, before the
  first `set`. B3 also needs a child-token store, so `start_home_relay` will
  take one more argument. It has one caller (`setup_router`), so that change
  is small.

Unit tests:

- `no_url_is_degraded_mode`: `from_config(&IrohParentConfig { url: None })`
  is `Ok(None)`.
- `bad_url_stops_startup`: `"localhost:7964"` gives an error whose text
  contains `parent_coordinator.iroh.url` and the value.
- `url_is_a_fixed_source`: a good URL gives `Ok(Some(Fixed { url }))`.

### Step 7 — `crates/substrate`: the record publisher reads the watch channel

File `crates/substrate/src/runtime/publish.rs`:

1. `publish_to_community_registry`: replace the parameter
   `relay_url: Option<String>` with
   `mut relay_rx: watch::Receiver<Option<RelayUrl>>` (same position, so it
   stays at 7 parameters). The loop reads the URL fresh before each pass, and
   the `select!` gets one more arm (D11):

   ```rust
   loop {
       // `borrow_and_update` marks the value seen, so the `changed` arm below
       // fires only for a newer relay.
       let relay_url = relay_rx.borrow_and_update().as_ref().map(ToString::to_string);
       let result = publish_self_and_hosted(&service_id, &endpoint_addr, relay_url, ...).await;
       if let Some(reply) = pending_reply.take() {
           let _ = reply.send(result);
       }
       tokio::select! {
           () = time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)) => {},
           Some(reply) = force_rx.recv() => pending_reply = Some(reply),
           // A new confirmed home relay: publish the record again now. If the
           // sender is gone this pattern does not match, so the arm is
           // disabled for this `select!` and cannot spin.
           Ok(()) = relay_rx.changed() => {},
       }
   }
   ```

   Do not hold the `borrow_and_update()` guard across an `.await` (the
   `.map(..)` ends it in the same statement).

2. `publish_self_and_hosted` skips the substrate's own record when
   `relay_url` is `None` (D4), and still replays the hosted records:

   ```rust
   let Some(relay_url) = relay_url else {
       debug!("no confirmed home relay yet; not publishing this substrate's endpoint record");
       replay_hosted(publisher).await;
       return Err(anyhow::anyhow!(
           "this substrate has no home relay yet, so its endpoint record was not published"
       ));
   };
   ```

   Move the two hosted calls (`publish_all_services` and
   `warn_on_near_expiry_records`) into a small helper `replay_hosted` and call
   it from both paths, rather than copying them. `build_signed_endpoint_info`
   keeps taking `Option<String>` and receives `Some(relay_url)`. The error
   reaches only a forced caller (`republish_now`); the heartbeat ignores it,
   as today. Log the skip at `debug`: degraded mode already warned at boot,
   and the fixed-relay task warns while it waits.

3. New function, moved out of `setup_router`:

   ```rust
   /// Starts the heartbeat that publishes this substrate's own endpoint
   /// record. The heartbeat also runs on a forced republish request and when
   /// the home relay changes.
   pub(super) fn start_self_record(
       config: &SubstrateConfig,
       service_id: &str,
       secret_key: [u8; 32],
       router: &ConnectionRouter,
       control_plane: &ControlPlaneService,
       publisher: Arc<EndpointPublisher>,
   ) {
       let (Some(endpoint_addr), Some(home_relay)) = (router.endpoint_addr(), router.home_relay())
       else {
           return;
       };
       let (force_tx, force_rx) = mpsc::channel(4);
       control_plane.set_republish_trigger(force_tx);
       publish_to_community_registry(
           service_id.to_string(), endpoint_addr, home_relay.subscribe(), secret_key,
           config.identity.nickname.clone(), publisher, force_rx,
       );
   }
   ```

   `set_republish_trigger` is still called synchronously during setup, as
   today. The first heartbeat pass runs at once. With a fixed relay it usually
   finds no confirmed relay yet, skips the substrate record and replays the
   hosted records. When the relay is confirmed, the `changed` arm runs a
   second pass that publishes the substrate record. Boot waits for neither.

File `crates/substrate/src/runtime/router.rs`, function `setup_router`:

1. Near the top of `setup_router`, before the connection router is built,
   read the source so a bad URL stops startup (D2). (`RuntimeServices::init`
   runs earlier; on a node with a coordinator role, that role's own check in
   step 8 reports the same bad URL first.)

   ```rust
   let relay_source = HomeRelaySource::from_config(&config.parent_coordinator.iroh)?;
   ```

   Right after `ConnectionRouter::init(..)`, start it:

   ```rust
   if let Some(handle) = router.home_relay() {
       home_relay::start_home_relay(handle, relay_source);
   }
   ```

   This runs whether or not a registry is configured: the endpoint needs its
   home relay either way. When `iroh` is not in `communication_interfaces`,
   there is no handle and nothing is logged.
2. Replace the block `if let Some(endpoint_addr) = router.endpoint_addr() { .. }`
   (lines 215-236 today, including its comment) with one call:

   ```rust
   publish::start_self_record(
       config, service_id, secret_key, &router, &control_plane, publisher.clone(),
   );
   ```

   Keep the existing comment about wiring `republish_now`; move it onto
   `start_self_record`'s doc comment if it reads better there.
3. Update the imports (`publish::start_self_record`, `home_relay`). Remove the
   `publish_to_community_registry` import from `router.rs` if it is no longer
   used there.

The relay URL is no longer read from `config.parent_coordinator.iroh` in
`router.rs` except through `HomeRelaySource::from_config`.

### Step 8 — `crates/coordinator_iroh`: no n0, no boot wait, one relay resolution

File `crates/coordinator_iroh/src/coordinator.rs`:

1. At the very start of `CoordinatorIroh::init`, before any relay server or
   listener starts:

   ```rust
   let parent_relay_url = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)?;
   ```

2. `resolve_relay_url` takes `parent_relay_url: Option<&RelayUrl>` and returns
   `anyhow::Result<Option<RelayUrl>>`:
   - a parent relay: return it;
   - relay off: `Ok(None)`;
   - own relay: build the string from the server address as today and parse
     it with `net_iroh::parse_relay_url("roles.coordinator.iroh.http_bind_address", ..)`.

   Update its doc comment.
3. In `init`, inside `if let Some(iroh_cfg) = &role.iroh`, resolve **once**,
   right after the relay server is spawned:

   ```rust
   let relay_url = resolve_relay_url(parent_relay_url.as_ref(), &relay_server, iroh_cfg)?;
   ```

   Pass this one value to the three users:
   - `Self::build_iroh_endpoint(iroh_cfg, relay_url.clone())` (the `config`,
     `_role` and `relay_server` parameters go away);
   - `Self::spawn_http_info_server(config, iroh_cfg, &iroh_endpoint,
     relay_url.clone(), parent_relay_url.clone(), active_conns)`: `relay_url`
     replaces `relay_server`, the unused `_role` goes away, and the parent URL
     is passed in for `InfoState.parent_coordinator_url`. Keep it at 7
     parameters or fewer;
   - the registry registration in step 4 of `init`:
     `relay_url.as_ref().map(ToString::to_string)` as `relay_url_payload`.

   `spawn_iroh_router` gets the parent URL from the caller too
   (`parent_relay_url.as_ref().map(ToString::to_string)` for
   `RouteHandler::new_coordinator`), instead of reading the config again.
4. `build_iroh_endpoint` (the method, ~l.193):

   ```rust
   let iroh_endpoint = net_iroh::build_iroh_endpoint(
       relay_url.clone(), Some(secret_key.clone()), iroh_cfg.idle_timeout_secs,
   ).await?;
   if relay_url.is_some() {
       log_when_online(iroh_endpoint.clone());
   } else {
       info!("coordinator Iroh endpoint has no relay; it dials peers through their relays");
   }
   ```

   Delete the blocking `timeout(Duration::from_secs(30), iroh_endpoint.online())`
   and the "Registering with parent coordinator" log (log the parent URL once
   in `init` instead). New private function:

   ```rust
   /// Logs when the endpoint reaches its relay. Boot never waits for it.
   fn log_when_online(endpoint: Endpoint) {
       tokio::spawn(async move {
           tokio::select! {
               res = time::timeout(ONLINE_LOG_TIMEOUT, endpoint.online()) => match res {
                   Ok(()) => debug!("coordinator Iroh endpoint is online"),
                   Err(_) => warn!("coordinator Iroh endpoint did not reach its relay within {ONLINE_LOG_TIMEOUT:?}"),
               },
               () = endpoint.closed() => {}
           }
       });
   }
   ```

   with `const ONLINE_LOG_TIMEOUT: Duration = Duration::from_secs(30);`.
   Remove imports that are no longer used.
5. `spawn_http_info_server`: build the payload from `relay_url`, not from
   `iroh_endpoint.addr()`:

   ```rust
   let endpoint_addr_payload =
       EndpointAddr::from_parts(iroh_endpoint.id(), relay_url.clone().map(TransportAddr::Relay));
   ```

   `InfoState.relay_url` is `relay_url.map(|u| u.to_string())` (D3). Shorten
   the long comment above the payload: keep why direct addresses are left out
   (they cannot be dialed between two processes that share only a relay, and
   link-local addresses lose their scope id). Add one sentence: the relay
   address comes from the chosen URL, because boot does not wait for the
   endpoint to come online. Remove the words `Endpoint::builder(...)` from
   that comment (the source scan in step 3 skips comments, but the sentence is
   no longer needed).

AC-16 holds by construction: the coordinator reads only
`parent_coordinator.iroh.url`, its own relay, or nothing.

### Step 9 — `crates/coordinator_iroh`: strict `access`

File `crates/coordinator_iroh/src/config.rs`:

1. Import `CoordinatorAccess` instead of `AccessControl`.
2. New public function, replacing the `match &role.access` block:

   ```rust
   /// Builds the relay access check from `[roles.coordinator] access`.
   ///
   /// # Errors
   /// For the passport forms, which this build cannot enforce yet, and for a
   /// list entry that is not an Iroh endpoint id.
   pub fn relay_access(access: &CoordinatorAccess) -> Result<AccessConfig> {
       match access {
           CoordinatorAccess::Everyone => Ok(AccessConfig::Everyone),
           CoordinatorAccess::List(ids) => restricted_to(ids),
           CoordinatorAccess::Passport => bail!(
               "[roles.coordinator] access = \"passport\" is not supported yet; \
                use \"everyone\" or a list of endpoint ids"
           ),
           CoordinatorAccess::ListOrPassport(_) => bail!(
               "[roles.coordinator] access = {{ list = [...], passport = true }} is not \
                supported yet; use the plain list form access = [...]"
           ),
       }
   }

   fn restricted_to(ids: &[String]) -> Result<AccessConfig> {
       let list = ids
           .iter()
           .map(|s| s.parse::<EndpointId>()
               .with_context(|| format!("[roles.coordinator] access: {s:?} is not an Iroh endpoint id")))
           .collect::<Result<Vec<_>>>()?;
       let list = Arc::new(list);
       Ok(AccessConfig::Restricted(Box::new(move |endpoint_id| {
           let list = list.clone();
           Box::pin(async move { if list.contains(&endpoint_id) { Access::Allow } else { Access::Deny } })
       })))
   }
   ```

   In `build_relay_config`: `let access_config = relay_access(&role.access)?;`.
   There is no `_ => AccessConfig::Everyone` arm any more.

File `crates/coordinator_iroh/src/coordinator.rs`, `CoordinatorIroh::init`:
right after the parent URL check from step 8.1, as the first line inside
`if let Some(role) = &config_roles {`, add

```rust
// Checked even when the relay is off, so a value this build cannot enforce
// stops the coordinator at startup instead of waiting unused.
relay_access(&role.access)?;
```

(import `crate::config::relay_access`). The WebRTC coordinator does not read
`access`, so a WebRTC-only role is not checked; the field is documented as
the Iroh relay's setting.

Unit tests in the `mod tests` of `config.rs` (the existing two tests keep
compiling; they use `..Default::default()`). `AccessConfig` has no
`PartialEq`, so assert with `matches!`:

- `everyone_admits_any_endpoint`:
  `matches!(relay_access(&Everyone)?, AccessConfig::Everyone)`.
- `list_admits_only_listed_endpoints`: build two ids from
  `SecretKey::generate(&mut rand::rng()).public()`; list one;
  `is_allowed(listed).await` is true, `is_allowed(other).await` is false.
- `list_with_bad_id_is_an_error`: error text contains "not an Iroh endpoint id".
- `passport_access_is_an_error` and `table_access_is_an_error`: error text
  contains "not supported yet".
- `build_relay_config_rejects_passport_access`: relay on, access passport,
  `build_relay_config` returns an error.

Other callers of the old type:

- `crates/smoke-tests/src/main.rs:96`: the `CoordinatorRole` literal uses
  `access: CoordinatorAccess::Everyone`. Line 87 (`ServiceRegistryRole`) is
  unchanged.
- `crates/coordinator_iroh/tests/multi_hop_relay.rs` and
  `crates/community_registry/src/registry/tests.rs`: their `AccessControl`
  uses are on `ServiceRegistryRole`; no change. Fix any `CoordinatorRole`
  literal the compiler reports.

### Step 10 — `crates/substrate/src/main.rs`: dev profile and CLI

1. Add `const DEV_RELAY_URL: &str = "http://localhost:7964";` near
   `dev_mode_config`, with a comment: the dev profile runs its own Iroh relay
   on the coordinator's default `http_bind_address`, and uses it as the home
   relay.
2. `dev_mode_config`: `c.parent_coordinator.iroh = Some(Default::default());`
   becomes `c.parent_coordinator.iroh.url = Some(DEV_RELAY_URL.to_string());`.
3. `--iroh-relay-url`: `config.parent_coordinator.iroh.url = Some(url);`.
4. `test_config_precedence`: the TOML uses the key `relay_url`, which the
   config ignores, so the test never checked the config-file value. Change it
   to `url = "http://config.relay:3340"`. The assertion becomes
   `assert_eq!(config.parent_coordinator.iroh.url.as_deref(), Some("http://cli.relay:3340"), ..)`.
5. New test `dev_mode_config_sets_the_local_relay`:
   `dev_mode_config().parent_coordinator.iroh.url.as_deref() == Some(DEV_RELAY_URL)`.

### Step 11 — configs that relied on the old meaning

Rust (compile errors point at them; replace
`config.parent_coordinator.iroh = Some(IrohParentConfig { url: X })` with
`config.parent_coordinator.iroh.url = Some(X)`, and drop the unused
`IrohParentConfig` import):

- `crates/substrate/tests/basic_lifecycle.rs:239`
- `crates/substrate/tests/podman_lifecycle.rs:101`
- `crates/substrate/tests/common/mod.rs:287`
- `crates/substrate/tests/common/node.rs:250`
- `crates/coordinator_iroh/tests/multi_hop_relay.rs:361`

Node configs with `iroh` in `communication_interfaces` but no
`[parent_coordinator.iroh]` had no Iroh endpoint before this change. After it
they would boot in degraded mode with a loud warning. Change both to
`communication_interfaces = ["webrtc"]`, which keeps today's behaviour
exactly (WebRTC without a `[parent_coordinator.webrtc]` section does nothing
in the router):

- `crates/substrate/tests/e2e/global-setup-multihop.ts`, the `configC` block
  (~l.159-161), node `C`.
- `docs/demo/demo.md` (~l.230), the `cloud` node config.

All other e2e configs set `[parent_coordinator.iroh] url`; no change.

`crates/substrate/config.sample.toml`: change the comment above
`[parent_coordinator.iroh] url` to say it is optional, what happens without
it (degraded mode), and that a value that is not an `http`/`https` URL with a
host stops startup. Change the comment above `[roles.coordinator] access` to
list the forms: `"everyone"`, a list of endpoint ids; `"passport"` and
`{ list = [...], passport = true }` are reserved and stop the coordinator
today; any other value is a config error.

Search once more before you finish:
`grep -rn "parent_coordinator.iroh\|IrohParentConfig\|coordinator_discovery_url" crates apps docs --include=*.rs --include=*.ts --include=*.toml --include=*.md`.
Ignore `docs/archive` and `scratch-notes` files.

### Step 12 — harness extension for the substrate tests

File `crates/substrate/tests/common/node.rs` (extend the shared harness; do
not write a new `Node` or `boot`):

1. `NodeBuilder` gets `wait_for_ready: bool` (default `true`) and

   ```rust
   /// Do not dial the node at boot. For a node nobody can reach yet, such as
   /// one with no confirmed home relay. The owner client is still built.
   pub fn skip_ready_wait(mut self) -> Self { self.wait_for_ready = false; self }
   ```

   In `boot_at`, call `wait_for_ready` only when the flag is set. Panic with a
   clear message if `inject_kek` is set together with `skip_ready_wait` (the
   KEK injection needs the connection).
2. Before `runtime` moves into the spawned task, take
   `let home_relay = runtime.connection_router.home_relay();`. Store it in a
   new `SubstrateNode` field `home_relay: Option<HomeRelay>`, and add

   ```rust
   /// This node's home-relay handle. Panics when the node has no Iroh endpoint.
   pub fn home_relay(&self) -> &HomeRelay
   ```

   `HomeRelay` is `syneroym_router::HomeRelay`. `syneroym-router` is a normal
   dependency of `syneroym-substrate`, but integration tests can only use the
   crate's dev- and normal dependencies by name if it is listed; if the import
   does not resolve, add `syneroym-router.workspace = true` to
   `[dev-dependencies]`.

New file `crates/coordinator_iroh/tests/common/mod.rs` (the one place where
`mod.rs` is allowed, see `AGENTS.md` module layout) with one helper:

```rust
/// Waits until the coordinator's endpoint lists its relay, as `init` used to
/// do before it returned. For tests that dial the coordinator through its
/// relay right after `init`.
pub async fn wait_for_relay(coord: &CoordinatorIroh, timeout: Duration) -> anyhow::Result<()>
```

It polls `coord.endpoint_addr()` every 50 ms until `relay_urls()` is not
empty, or fails after `timeout`.

### Step 13 — tests

**13a. iroh order and the handle with real relays** — extend
`crates/coordinator_iroh/tests/relay_map_runtime.rs`. Reuse its
`spawn_relay`, `bind_empty`, `wait_for_relays`, `spawn_echo` and
`dial_via_relay_only` helpers. Update the `//!` header to say the file also
covers `HomeRelay`. Add a `dead_relay()` helper here too, the same shape as
in step 4 (a `TcpListener` on `127.0.0.1:0` that never accepts). It is a few
lines; if the duplication gate flags it, keep one copy in the router crate as
a `pub` item behind a `test-util` feature instead.

- `inserting_new_relay_then_removing_old_moves_home_relay` (iroh only): insert
  `r1`, wait for `[r1]`; insert `r2`; remove `r1`; wait for `[r2]`; assert it;
  a second endpoint dials `a` through `r2` only and gets the echo.
- `home_relay_set_confirms_relay_and_updates_watch`:
  `HomeRelay::new(bind_empty(), HOME_RELAY_CONFIRM_TIMEOUT)`; subscribe;
  `set(r1)` is `Ok`; the receiver has changed and holds `Some(r1)`;
  `a.addr()` lists `r1`; a peer dials through `r1`. Then `set(r1)` again is
  `Ok` and the receiver has *not* changed (`has_changed()` is false).
- `home_relay_switch_moves_home_relay_and_updates_watch`: `set(r1)` then
  `set(r2)`; the watch holds `Some(r2)`; `a.addr()` lists only `r2`; a peer
  dials through `r2`.
- `home_relay_switch_to_a_dead_relay_rolls_back`: one handle with
  `HOME_RELAY_CONFIRM_TIMEOUT` (a confirmed switch can take about 10 s, so a
  shorter timeout would fail on a slow machine; the dead-relay step then takes
  15 s, which is acceptable). Do not build a second handle on the same
  endpoint: its `in_map` would differ. `set(r1)` is `Ok`; `set(dead)` is
  `NotConfirmed`; the watch still holds `Some(r1)`; `a.addr()` lists `r1`; a
  peer dials through `r1`.
- `home_relay_set_after_a_rolled_back_switch_works`: the same handle and
  endpoint, continuing the scenario: `set(r2)` is `Ok`; the watch holds
  `Some(r2)`; a peer dials through `r2`.

In `home_relay_switch_moves_home_relay_and_updates_watch`, measure the time of
the first `set(r1)` and of the switch `set(r2)`, and print both with
`eprintln!`, as the spike does. Record the times you see (local and CI) in the
change doc `## Deviations`, next to D10.

**13b. coordinator startup** — new file
`crates/coordinator_iroh/tests/startup_config.rs`. Use `127.0.0.1:0` for every
bind address (`http_bind_address`, `quic_bind_address`,
`info_http_bind_address`); with the relay off they bind nothing fixed. Set
`config.substrate.enable_bep0044_dht = false` in every config: outside
`syneroym-core`'s own unit tests the default is `true` (`base.rs:233`), and
`/v1/info` does a registry lookup.

- `coordinator_without_parent_or_own_relay_starts_with_no_relay`: role with
  `iroh = { enable_relay = false }`, no parent URL.
  `time::timeout(Duration::from_secs(10), CoordinatorIroh::init(&config))`
  succeeds. After 1 s, `coord.endpoint_addr()` lists no relay URL.
  `GET /v1/info` (at `coord.info_addr()`) returns `relay_url: null`. Shut the
  coordinator down.
- `coordinator_with_passport_access_stops_at_startup`: `access =
  CoordinatorAccess::Passport`, relay off. `CoordinatorIroh::init` returns an
  error whose text contains "not supported yet".
- `coordinator_with_bad_parent_url_stops_at_startup`:
  `parent_coordinator.iroh.url = Some("localhost:7964".into())`.
  `CoordinatorIroh::init` returns an error whose text contains
  `parent_coordinator.iroh.url`.

**13c. substrate end-to-end** — new file
`crates/substrate/tests/home_relay_e2e.rs`. Use `SubstrateNode::builder()`
and hold `common::serial_guard().await` for each test body. Ports come from
the harness (`alloc_ports` inside `NodePorts::alloc`). No port literals.

Helpers in the file:

```rust
/// The relay URL in `node`'s published record. `None` when no record is
/// published yet.
async fn record_relay_url(node: &SubstrateNode) -> Option<String>

/// Polls `record_relay_url` until it equals `want`, or panics after `timeout`.
async fn wait_for_record_relay(node: &SubstrateNode, want: &str, timeout: Duration)
```

`record_relay_url` calls `node.substrate_client.lookup().await`; a lookup
error (no record) gives `None`, otherwise it returns the `relay_url` of the
`EndpointMechanism::Iroh` entry. Poll every 200 ms. Compare relay URLs in
their `RelayUrl` form: `url.parse::<RelayUrl>()?.to_string()`.

- `a_substrate_with_no_relay_boots_and_takes_a_relay_set_later`:
  - build with `.configure(|c| c.parent_coordinator.iroh.url = None)` and
    `.skip_ready_wait()`, and boot;
  - wait 3 s (longer than one heartbeat pass plus a registry round trip on
    loopback); still no substrate record is published (D4);
  - `node.home_relay().set(node.relay_url().parse()?)` is `Ok` (the node runs
    its own relay through the coordinator role);
  - `wait_for_record_relay(&node, node.relay_url(), 20 s)`;
  - `node.substrate_client.wait_for_ready(Duration::from_secs(30))` succeeds:
    peers now reach the node through that relay.
- `a_relay_that_is_down_at_boot_is_used_once_it_comes_up`:
  - `let builder_b = SubstrateNode::builder();` do not boot it yet;
    `let b_relay = format!("http://127.0.0.1:{}", builder_b.ports().iroh);`
    (the harness holds that port until `b` boots, so nothing else takes it);
  - boot `a` with `.shared_relay(b_relay.clone()).skip_ready_wait()`. This
    boot finishing at all proves boot does not wait for the relay;
  - wait 3 s; `a` has no substrate record (D4);
  - boot `b`;
  - `wait_for_record_relay(&a, &b_relay, 60 s)` (iroh's own report runs every
    20 to 26 s, plus one confirm wait);
  - `a.substrate_client.wait_for_ready(Duration::from_secs(30))` succeeds.
- `switching_the_home_relay_republishes_the_record`:
  - boot node `a` and node `b` with the default builder (each uses its own
    relay as the fixed `url`);
  - `a`'s record names `a.relay_url()`;
  - `a.home_relay().set(b.relay_url().parse()?)` is `Ok`;
  - `wait_for_record_relay(&a, b.relay_url(), 20 s)`;
  - reconnect `a.substrate_client` (`shutdown()` then `connect()`) and make
    one `orchestrator.readyz` request: it succeeds through `b`'s relay.

**13d. existing tests that change, and why**

| Test | Change | Why |
| --- | --- | --- |
| `crates/substrate/src/main.rs` `test_config_precedence` | TOML key `relay_url` → `url`; assertion on `iroh.url` | Shape change; the old key was silently ignored |
| `crates/coordinator_iroh/src/config.rs` two `build_relay_config` tests | none expected | They use `..Default::default()`; default access is still `Everyone` |
| `crates/substrate/tests/{basic_lifecycle,podman_lifecycle}.rs`, `common/{mod,node}.rs`, `crates/coordinator_iroh/tests/multi_hop_relay.rs` | `iroh.url = Some(..)` | `iroh` is no longer an `Option` |
| `crates/coordinator_iroh/tests/connection_limit.rs:128,200` | call `common::wait_for_relay(&coord, 30 s)` after `CoordinatorIroh::init` (add `mod common;`) | `init` no longer waits for the endpoint to be online, and the test dials the coordinator through its relay at once |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs` | call the same helper after the `CoordinatorIroh::init` calls at 323 and 362, so both coordinators are online before the `sdk_client.connect()` calls at 595 and 638 (add `mod common;`) | same |
| `crates/smoke-tests/src/main.rs` | `CoordinatorAccess::Everyone` | New type |
| `crates/substrate/tests/e2e/global-setup-multihop.ts` node `C` | drop `iroh` from `communication_interfaces` | It relied on "no section means no Iroh endpoint" |
| `crates/coordinator_webrtc/src/bootstrap.rs` test `test_state` | none | `build_iroh_endpoint(None, ..)` still compiles, now with an empty map |

Watch for tests that look a node up in the registry right after boot,
without `wait_for_ready`. With D4 the substrate record appears about a second
after boot instead of at once, so such a lookup may now find no record. If
one becomes flaky, the fix belongs in the test (wait for the record), not in
a longer boot.

## 4. Things to check while you work

- **Function size.** `setup_router` gets shorter. `ConnectionRouter::init` and
  `CoordinatorIroh::init` must stay under the 100-line limit; extract helpers
  if they grow. Match arms stay under ~10 lines.
- **Arguments.** No function above 7 parameters (clippy
  `too_many_arguments`).
- **File size.** All touched production files stay well under 800 lines.
- **Duplication.** Run the `cargo dupes report` recipe from `AGENTS.md` over
  your files. If the duplication gate says the number got better, lower
  `MAX_EXACT_DUPLICATE_PERCENT` in `xtask/src/duplication.rs` to the value it
  names.
- **Comments.** Explain why, in simple English. No planning ids (the
  `planning-refs` gate in `mise run verify` runs
  `.github/scripts/check-planning-refs.py`), no slice or milestone names, no
  decision ids such as "D2", no line counts.
- **Imports.** Types by `use`; functions qualified by their parent module;
  no inline paths with several `::` (`AGENTS.md`, Mandatory Import Cleanup).
- **`cargo shear`.** The new `toml` dev-dependency of `syneroym-core` and
  `thiserror` of `syneroym-router` must both be used.

## Completion steps

The implementer must follow these steps.

1. **Verify.** Run `mise run verify` as a background job (it takes about 20
   minutes) and read its summary when it finishes. Every gate must pass. Do
   not use `--skip` in the final run. If a gate fails, fix the cause and run
   the full command again. While you iterate, targeted commands are fine, for
   example `cargo nextest run -p syneroym-router`,
   `cargo nextest run -p syneroym-coordinator-iroh`,
   `cargo nextest run -p syneroym-substrate --test home_relay_e2e`.
2. **Living docs.**
   - `docs/system-architecture.md`:
     - the config table row `[parent_coordinator.iroh] url` (~l.351): the
       setting is optional, no default; without it the substrate starts in
       degraded mode; a value that is not an `http`/`https` URL with a host
       stops startup; the Iroh endpoint exists when `communication_interfaces`
       has `iroh`;
     - the row `[roles.coordinator] access` (~l.365): the four forms, which
       two work today, and that other values stop startup. Remove "Any other
       string also means everyone";
     - the dev-setup sentence (~l.368): the dev profile sets
       `parent_coordinator.iroh.url = "http://localhost:7964"` explicitly;
     - the "Default relay" paragraph (~l.372): rewrite. No n0 preset at run
       time; one relay or an empty map; the substrate sets its home relay at
       run time, publishes no record of its own until a relay is confirmed,
       and publishes again when the relay changes; boot does not wait for the
       endpoint to come online;
     - the relay paragraph (~l.280): the `access` forms and the strict check;
     - the "Envisioned" list (~l.374) and note (~l.1741): remove
       `coordinator_discovery_url`;
     - the multi-hop walk-through (~l.1733, ~l.1739): Cp no longer waits 30 s
       at startup; a substrate has an Iroh endpoint when
       `communication_interfaces` has `iroh`.
   - `docs/system-requirements-spec.md`:
     - the "Home Relay / Coordinator Relay" definition (~l.160): `url` is
       optional, and the home relay can change at run time;
     - the Connectivity section (~l.267): it cites `net_iroh.rs:94` and
       `coordinator.rs:180`; both lines move. Point the citations at the new
       lines.
   - `docs/developer-guide.md`: a grep at planning time found no mention of
     these settings. Check again; update only if it lists them.
   - `docs/TERMINOLOGY.md`: add rows "Home relay" and "Degraded mode" to the
     "Component names (engineering)" table, with the meanings from §0 of this
     plan. The "Home relay" row must match the spec's definition above.
   - `docs/demo/demo.md`: the `cloud` node config (step 11).
   - `docs/planning/changes/registry-listed-coordinators/change.md`: set
     `status: in-progress`; list the living docs you changed under
     `living-docs-touched`; fill the Test column for AC-1, AC-10 and AC-14 with
     the test names; write in `## Deviations` every difference between the
     code and the change doc's Design and criteria: at least D2 (a bad URL
     stops startup, which changes the AC-10 text), D3, D4 (no substrate record
     without a relay URL), D5 (rollback), D6, D10 (confirm timeout and QAD),
     D11 (wake on relay change instead of pushing into the trigger channel),
     the note that "confirmed" does not prove access (for slice B3), and any
     change you had to make to this plan; tick only the Tasks that are fully
     done ("Remove both n0 fallbacks", "Write the dev profile relay
     explicitly").
   - `docs/planning/changes/operator-passport/change.md`: fill the Test
     column for H-6. Add one sentence under "Substrate": a relay confirmed as
     the home relay may still refuse the node, so registration success, not
     relay confirmation, is the proof of access. Do not change its status.
3. **Status and backlog.**
   - `docs/planning/milestones/M06D-reachability-and-deferred-delivery/status.md`:
     split the "A1 to A4, …" row so A1 has its own row. Status "Done"; the
     tool that implemented it; evidence: the `mise run verify` result and the
     new test files and test names.
   - `docs/planning/deferred-backlog.md`:
     - `[roles.community_registry] access` is parsed into the lenient
       `AccessControl` type and no code reads it: add it to the row "Config
       flags and options that nothing reads" (~l.268), and fix that row's
       citation `base.rs:197-198`, which moves after step 1;
     - the D2 gap: until slices A4 and B3, nothing outside tests sets the home
       relay at run time, so a degraded node stays degraded until it restarts
       with a `url` (target A4, B3);
     - QAD port mismatch (new row): `RelayConfig::from(url)` probes QAD on
       UDP 7842 (`iroh-relay-0.97.0/src/defaults.rs:7`), but our coordinator
       serves QAD on `quic_bind_address` (default 7965) and only with TLS
       (`crates/coordinator_iroh/src/config.rs:71-79`). So QAD never works
       today, and each net report waits for the probe timeout. Fix: give
       relays a QAD port the client can learn, or turn QAD off for relays that
       do not serve it (target A4, which brings third-party relays);
     - any `TODO`/`FIXME` you leave in the code, in the "Open in-code markers"
       section.
4. **Stop.** Do not start slice A2. If you are not making progress after a
   reasonable number of attempts, stop and report what blocks you, with the
   failing output.
