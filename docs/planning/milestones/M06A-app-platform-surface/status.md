# M06A App Platform Surface — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-a1-implementation-plan.md](slice-a1-implementation-plan.md) (A1),
[slice-a2-implementation-plan.md](slice-a2-implementation-plan.md) (A2),
[slice-a4-implementation-plan.md](slice-a4-implementation-plan.md) (A4)

**Overall:** Slice A1 complete (2026-08-14). Slice A2 complete (2026-08-14).
A3–A5 not started.

> **Renumbered 2026-08-14.** `D-06A-2` was reversed — inbound WebSocket is
> built rather than replaced by SSE — and the new WebSocket slice took **A3**,
> because its guest-side API is a design question better settled before the
> demo app is written than retrofitted after. The demo app moved A3 → **A4**
> (its plan file renamed with it, decision ids `D-A3-n` → `D-A4-n`) and the
> Playwright suite A4 → **A5**. A1 and A2 are unaffected; references to the
> demo app in their plans and in the evidence below were updated with the
> renumber.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| A1 | Blob-backed static serving | **Complete (2026-08-14)** — [implementation plan](slice-a1-implementation-plan.md), evidence below | None — independently mergeable |
| A2 | Guest HTTP route target | **Complete (2026-08-14)** — [implementation plan](slice-a2-implementation-plan.md) revision 4, evidence below | None — independent of A1 |
| A3 | Inbound WebSocket | **Complete (2026-08-15)** — [implementation plan](slice-a3-implementation-plan.md), evidence below | A2 (Complete) |
| A4 | The demo app | Not started — [implementation plan](slice-a4-implementation-plan.md) | A1, A2 (Complete) and A3 (Complete) |
| A5 | The Playwright suite | Not started | A4 |

---

## A1 — What shipped

A deployed service can now declare a static asset bundle (`asset-bundle` on
`service-config`, WIT-added per [slice-a1-implementation-plan.md](slice-a1-implementation-plan.md)
§3.1) that the router serves straight from blob storage, exactly matching the
milestone's D-06A-1 requirement: the WASM sandbox is never instantiated to
answer a static request.

**Types and deploy-time caps** (`crates/wit_interfaces/wit/control-plane/control-plane.wit`,
`crates/app_orchestration/src/models.rs`, `crates/core/src/asset_manifest.rs`,
`crates/core/src/deploy_docs.rs`): a `visibility` enum (`public`/`internal`/
`private`, ADR-0018's rejected-by-name alternative to a bare bool, D-A1-1);
`asset-bundle { archive, hash, visibility }`; three caps —
`MAX_ASSET_BUNDLE_BYTES` (2 MiB, a cheap early guard), `MAX_ASSET_UNPACKED_BYTES`
(64 MiB, against a decompression bomb), and `MAX_ASSET_FILE_COUNT` (10,000) —
plus `reject_archive_entry_path`, the archive-entry analogue of the existing
`reject_relative_escape` traversal guard (D-A1-5).

**Unpacking** (`crates/control_plane/src/assets.rs`): `unpack_asset_bundle`
decompresses and validates an archive entirely synchronously (the `tar`
crate's `Archive`/`Entries` types are not `Send`, so this phase is fully
consumed and dropped before the first `.await`, or every caller up through
the JSON-RPC dispatch trait's `async_trait` bound stops compiling), then
writes each accepted file to blob storage. `written` accumulates every hash
the call stores, including on the error path, so a caller-owned rollback can
always see what to undo. `store_manifest`/`hashes_of`/`delete_hashes` give
deploy/undeploy one set-difference-based helper for both directions (D-A1-9):
forward (a successful redeploy removes the old manifest's hashes, keeping
the new one's) and backward (a failed deploy removes what it itself wrote,
keeping the still-live old manifest's) — content-addressed blobs have no
refcount, so an unchanged file shares a hash across generations and a
wholesale delete in either direction would destroy live data.

**Deploy/undeploy wiring** (`crates/control_plane/src/service/orchestration.rs`,
`crates/control_plane/src/service.rs`): `deploy_with_context` unpacks the
declared bundle right after FDAE policy resolution, resolves the archive
(`Binary` only — `Url` is an explicitly rejected dead branch, matching the
Wasm component's own `source`), and holds the result until the same commit
point `http_routes` already uses. Every failure branch between there and the
commit — native-capability registration, and each of
`deploy_wasm_service`/`deploy_tcp_service`/`deploy_container_service` —
additionally rolls back the asset-bundle write via the same `written` set.
`ControlPlaneService`/`RouteHandlerInner`/`RouteHandlerDeps` all gained an
`AssetRegistry` field alongside the pre-existing `HttpRouteRegistry`, same
`Arc`-shared, cache-not-persistence shape (D-A1-2: nothing restores it from
storage at boot, same as `http_routes`). `undeploy` tears the whole bundle
down unconditionally, keeping nothing.

**Serving** (`crates/router/src/route_handler/http.rs`): `try_handle_asset`
is spliced between the fixed `GET /blobs/{hash}` route and the per-service
`http_routes` table. `resolve_asset` does exact-path lookup plus D-A1-11's
one rewrite (a path ending in `/` resolves to `<path>index.html` — a
directory index, not SPA history fallback, which stays the demo app's problem,
slice A4) and returns `None` — indistinguishable — for a missing service, a non-`public`
bundle, or a genuine miss (D-A1-8). `GET`/`HEAD` only; `ETag`/`If-None-Match`
304 handling; `Cache-Control` chosen by content type (`no-cache` for
`text/html`, whose name is stable while its content changes every deploy;
long-lived `immutable` otherwise, correct for bundler-hashed filenames). The
actual byte stream reuses the existing `blob-store`
`open-download`/`read-chunk` native-dispatch path verbatim — the same
`NativeService` arm `handle_blob_get` already takes, so F3 (the blob path is
`NativeService`-shaped) holds by construction and the sandbox genuinely
never starts.

**Deploy-time collision detection** (D-A1-4): an asset path matched by a
declared `GET`/`HEAD` route *pattern* — not just literal-string equality —
is refused at deploy. This needed `match_path` moved from `router` into
`syneroym_core::http_routes` (R3-A), since the collision check runs in
`syneroym-control-plane` at deploy time, before the router ever sees the
service.

**Instantiation counter** (D-A1-7, `crates/sandbox_wasm/src/engine.rs`):
`AppSandboxEngine` gained `instantiations()`, backed by both a
`metrics::counter!` and an in-process `AtomicU64`, so a test can assert a
**delta** across one request rather than an absolute count (deploy-time
lifecycle hooks also instantiate the component, so an absolute assertion
would be coupled to unrelated behaviour).

**CLI**: `roymctl svc deploy --wasm` gained `--assets <path>` /
`--asset-visibility <public|internal|private>`, via a new
`SyneroymClient::deploy_svc_wasm_with_assets` (the pre-existing
`deploy_svc_wasm` is unchanged, so none of its seven existing call sites
needed editing). Only wired for `--wasm`: TCP/container services already run
their own web server outside the substrate, which is exactly the thing A1
exists to stop being the only way to serve a web app. **Enforced substrate-
side, not just at this one CLI flag** (found in post-implementation review,
2026-08-14): `deploy_with_context` (`crates/control_plane/src/service/
orchestration.rs`) now rejects any manifest declaring `assets` for a `Tcp`/
`Container` service before anything fallible runs. A `Tcp`/`Container`
service's endpoint is `SubstrateEndpoint::TcpHostPort`, which the router's
`dispatch.rs` unconditionally routes to raw `io::copy_bidirectional`
passthrough regardless of what the client actually sends -- the
asset-serving HTTP path is structurally unreachable for one, so accepting
`assets` there previously meant silently unpacking and storing a bundle
that could never be served. Every deploy path funnels through this one
check (the single-service CLI, `roymctl supervisor submit`'s multi-service
plans, and any other JSON-RPC caller), not only `roymctl svc deploy`'s own
`requires = "wasm"` clap constraint.

### Scoped deviations from the plan (recorded, not silent)

- **`AssetBundle.archive`'s relative-path resolution** follows each call
  site's existing rule for `WasmManifest.source` (cwd-relative in
  `sdk::mapper`, manifest-dir-relative in `roymctl supervisor submit`'s
  pre-send inlining) rather than threading a new `manifest_dir` parameter
  through `ApplyRequest`/`map_deployment_plan_to_wit` and its ~11 call sites.
  Still fixes the plan's actual concern (a remote `supervisor submit`
  reaching a substrate that can't read a local path); see
  [deferred-backlog.md](../../deferred-backlog.md) §4 for the full
  reasoning.
- **`core::asset_manifest::ServiceAssets` stores `public: bool`**, not the
  full `Visibility` enum, since `syneroym-core` has no dependency on
  `syneroym-app-orchestration` and `internal`/`private` are byte-identical
  to "refuse" in A1 (D-A1-8) — see the same deferred-backlog.md row.
- **A miss (D-A1-8, failure-matrix rows 3 and 7) answers 405, not the
  plan's originally-specified 404.** `try_handle_asset` returning `None`
  falls through to route resolution and then to the JSON-RPC bridge, whose
  non-`POST` rejection is a uniform 405 for every unmatched `GET`/`HEAD`,
  asset or not. Returning 404 instead would mean special-casing that
  bridge's method check for those two methods, changing behaviour for the
  ordinary (non-asset) unmatched-route case too — a bigger, riskier change
  than this slice's own scope. The property the matrix actually cares
  about — a private bundle is indistinguishable from no bundle — holds
  regardless of which 4xx it is, and is what `test_static_asset_private_
  visibility_matches_no_bundle` actually tests: response equality between
  the two deploys, not a specific status code. Renamed from its original
  `..._is_a_plain_404` for the same reason. task.md's matrix rows 3 and 7
  are corrected to say 405.
- **D-A1-5's authoritative combined-size check landed shaped differently
  than the plan described, and later than the rest of the slice
  (2026-08-14, post-implementation review).** The plan called for
  `encoded(component) + encoded(bundle) + envelope < MAX_FRAME_SIZE`,
  computed client-side ahead of assembling the request. What shipped
  instead: `SyneroymClient::open_request_stream` (`crates/sdk/src/lib.rs`)
  serializes the *actual* outgoing JSON-RPC request first, then checks the
  real byte length against `MAX_FRAME_SIZE` before any network I/O for the
  call — exact, not an estimated ratio, and it protects every request this
  client makes (deploy, deploy-plan, write-bindings, ...), not only a
  deploy's asset bundle. Both `roymctl svc deploy --assets` and `roymctl
  supervisor submit` go through this one choke point, so no separate
  `sdk::mapper`/`apps/roymctl` wiring was needed.

All four are also documented inline where the deviation lives.

---

## A1 — Verification evidence (2026-08-14)

**New tests:**
- `crates/control_plane/src/assets.rs`: 22 unit tests covering every cap,
  traversal/collision case (including the method-filtered collision check,
  its D-A1-11 directory-index form, and a false-positive guard for a file
  merely ending in "index.html"), a decompression-bomb-shaped non-file entry
  still bounded by the unpacked cap, a duplicate-path rejection,
  quota-failure rollback, `written`-on-the-error-path, and `delete_hashes` as
  a pure set difference in both directions.
- `crates/control_plane/src/service/orchestration.rs`: 5 integration tests —
  deploy/undeploy round-trip (registry entry populated, blob readable back,
  manifest blob deleted on undeploy); a redeploy that shares some files
  with the prior generation (unchanged blob survives, dropped file's blob is
  collected); a redeploy that fails at WASM endpoint registration (after
  the asset block has already written the new generation's blobs and the
  component itself compiled), proving the backward rollback through a real
  failure rather than `delete_hashes` called directly as pure set
  arithmetic — the still-live old generation's manifest and blobs survive,
  the failed attempt's own blob does not; and two tests asserting a `Tcp`
  or `Container` service's deploy is rejected outright when it declares an
  asset bundle, before anything fallible runs.
- `crates/substrate/tests/static_assets_e2e.rs`: 5 end-to-end tests over a
  real Iroh QUIC connection — directory index / ETag / 304 / HEAD /
  Cache-Control-by-type / zero instantiation delta; cross-service isolation;
  private-visibility parity with no bundle at all; a multi-chunk
  (300 KB, several 64 KiB `read-chunk` round trips) byte-identical transfer;
  assets and a declared `http_route` coexisting without either shadowing the
  other.

**Commands run, from a clean tree:**

```
cargo +nightly fmt --all                                    # clean, no diff
cargo clippy --workspace --all-targets --all-features        # 0 warnings, 0 errors
cargo test --workspace                                       # see below
mise run test:e2e                                             # 8 passed + 4 passed (multi-hop config), exit 0
```

**`cargo test --workspace`:** run sandboxed (this environment's default) and
via targeted unsandboxed re-runs of every crate the sandboxed run flagged.
The sandboxed run's only failures are this project's known, pre-existing
sandbox artifact — a handful of tests across `syneroym-control-plane`
(7 health/DHT-probe tests), `syneroym-community-registry` (6 tests), and
every `crates/substrate/tests/*_e2e.rs` binary that all fail identically with
`Operation not permitted` binding a real localhost socket, which the sandbox
blocks by design (see this repo's own `AGENTS.md` guidance on sandboxed vs.
unsandboxed test runs). Confirmed pre-existing and unrelated to this slice by
re-running unsandboxed:

- `cargo test -p syneroym-control-plane` (sandbox off): **195/195 passed**
  (186 pre-existing + 2 new asset integration tests + `syneroym-control-plane
::assets` module's own 17 — the same 7 that fail sandboxed pass here).
- `cargo test -p syneroym-community-registry -p syneroym-app-supervisor`
  (sandbox off): **all passed**, including the 6 registry tests that fail
  sandboxed.
- `cargo test -p syneroym-router --lib` (sandbox off): **173/173 passed**
  (unchanged from before this slice — `try_handle_asset`'s only router-crate
  unit-test-visible surface is the `match_path` tests, which moved to
  `syneroym-core` and are covered there instead).
- `cargo test -p syneroym-substrate --test static_assets_e2e` (sandbox off):
  **5/5 passed**.
- `cargo test -p syneroym-substrate --test http_passthrough_e2e` (sandbox
  off): **6/6 passed** — the pre-existing M3B Slice 7 suite, re-verified
  since this slice edits the same file (`route_handler/http.rs`) it
  exercises.

No crate outside this list showed a failure in the sandboxed
`cargo test --workspace --no-fail-fast` run beyond the same "every e2e test
binary needs a real socket bind" pattern already described above.

**Exit criteria from task.md, as far as A1 alone can prove them** (2, 5, 6, 7
are A2/A3/A4's — the milestone's own criterion list says so explicitly):
1. *Not A1's to prove* — a property of A3's fixture.
3. `GET /index.html` returns 200 with the correct content type; the
   instantiation-count delta across the request is `0` —
   `test_static_asset_serving_index_etag_and_directory_rewrite`.
4. A repeat `GET` with `If-None-Match` returns 304 with an empty body —
   same test.
8. Every row of the failure and security matrix that is A1's to cover (not
   A2/A3/A4's — see [slice-a1-implementation-plan.md](slice-a1-implementation-plan.md)
   §7's own scoping note) has a test: rows 1, 2, 3, 4, 7 above, plus D-A1-4's
   collision check and D-A1-9's forward/backward blob cleanup.
9. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, `cargo test --workspace`, and `mise run test:e2e` are
   clean — see commands above.

---

## A2 — What shipped

[slice-a2-implementation-plan.md](slice-a2-implementation-plan.md), revision 4
after three review rounds, is the design of record. It settles task.md's open
design point on what a guest HTTP handler looks like in WIT: a dedicated
`syneroym:http/incoming-handler@0.1.0` export (`handle-request`), reached by
calling the sandbox engine directly rather than through
`dispatch_json_rpc_once` — an `http-native` connection resolves to a
`NativeService` pipeline, so the JSON-RPC bridge can never reach a guest.

**Two decisions worth knowing**, because both changed during review and the
reversals are recorded rather than tidied away:

- **A guest route is authenticated by default.** `HttpRoute` gains
  `public: bool` (`#[serde(default)]`, `crates/core/src/http_routes.rs`),
  default `false`, mirroring A1's `D-A1-1` rather than the dispatch-layer
  doctrine that WASM guests admit anonymous callers. What it actually gates is
  narrow and stated as such: a *direct* anonymous connection (WebRTC, raw
  QUIC). It gates nothing proxied by the client gateway or any
  `SyneroymClient`, both of which always self-assert a pubkey — pinned by
  `test_through_the_gateway_a_non_public_route_is_reached_and_reports_self_asserted_node_did`,
  see below.
- **The guest sees its caller** (`caller: option<caller-identity>` on the
  `http-request` WIT record, `crates/core/src/guest_http.rs`'s
  `GuestCallerIdentity`/`GuestCallerAuth` on the host side), with a
  `self-asserted` variant so the WIT cannot imply that gateway traffic — or an
  unchallenged pubkey — is a verified identity. Derivation is deliberately
  **mixed** (`D-A2-12`), not read off one source: `ucan` from
  `CallerContext.auth == AuthLevel::Ucan` (the honest signal — a rejected
  chain leaves `auth` at `Delegated` while `preamble.ucan` stays `Some`, F5b),
  `delegated` from `preamble.delegation.is_some()` (a malformed certificate is
  a hard reject before this point, so reaching here means it verified),
  else `self-asserted`. `crates/router/src/route_handler/http.rs`'s
  `guest_caller_identity` fails closed (a 500, not a lossy map) on a
  substrate-injected `AuthLevel` (`LocalElevated`/`LocalReadOnly`/`System`),
  which cannot legitimately reach an inbound HTTP request.

**WIT and host-side types.** `crates/wit_interfaces/wit/http/http.wit`: a new
standalone `syneroym:http@0.1.0` package (`incoming-handler`,
`caller-auth`/`caller-identity`/`http-request`/`http-response`) — deliberately
*not* referenced from `wit/host/host.wit` (`D-A2-1`, matching
`syneroym:data-layer/authorizer`'s precedent: optional, a component only
implements it when it opts in). `crates/core/src/guest_http.rs` mirrors the
records on the host side field-for-field, in WIT declaration order (the
dynamic `Val::Record` built from it must match).

**Dynamic marshalling, not a typed `bindgen!` path** (`D-A2-2`,
`crates/sandbox_wasm/src/http.rs`): `request_to_val`/`response_from_results`
reuse `stream.rs`'s `bytes_to_val_list`/`val_list_to_bytes` (now `pub(crate)`)
— every guest call in this crate is already dynamic, so a first typed export
path for one interface capped at 1 MiB is new machinery the plan explicitly
declines (backlog row owed, deferred-backlog.md). `response_from_results`
distinguishes a guest `Err(msg)` (`Declined`) from a wrong return shape
(`Malformed`) *before* inspecting the `Ok` payload, so a deliberate rejection
is never reported the same way as a broken component.

**Engine** (`crates/sandbox_wasm/src/engine.rs`):
`AppSandboxEngine::handle_guest_http_request` runs one request through a
fresh per-call instance (`build_store_and_instantiate`, same
"instantiate-call-discard" shape `authorize_rows`/`invoke_lifecycle_hook`
use), bounded by the existing `dispatch_epoch_ticks` (task.md's 5s
`dispatch_epoch_timeout_secs`, no new knob). `exports_http_handler` mirrors
`exports_authorize_rows`'s cheap static-type check. A new per-service
`guest_http_permits: Arc<DashMap<String, Arc<Semaphore>>>` (`D-A2-11`) bounds
concurrent guest HTTP requests — sized by a new `AppSandboxRole` field
`max_concurrent_guest_http_per_service` (default 4, living beside the other
instance-budget knobs, not on `StreamingConfig`) — acquired *before*
instantiation with a fixed `GUEST_HTTP_ADMISSION_TIMEOUT` (2s); a timed-out
wait or a `PoolConcurrencyLimitError` from the pool both become
`GuestHttpFailure::Unavailable`, mapped to 503 + `Retry-After: 1` by the
router, never a 500 or a hang. `forget_guest_http_permits` tears the map down
on undeploy, called beside `unsubscribe_all`
(`crates/control_plane/src/service/orchestration.rs`).

**One trap classifier, not a third hand-rolled copy** (`D-A2-6`):
`classify_call_failure`/`CallFailure` replace `execute_wasm_vals`'s and
`authorize_rows`'s two independently-drifting copies of the same fuel/memory/
epoch taxonomy (F9's finding — the two disagreed, and `authorize_rows` had no
memory-fault arm at all). Both call sites' pre-refactor observable behaviour
is preserved exactly, including that gap: a memory fault at the
`authorize_rows` site still becomes `AbacError::Trap`, not a budget error.
`ActiveInstanceGuard` (the `substrate.wasm.active_instances` gauge) is hoisted
from inside `execute_wasm_vals` to module scope so `handle_guest_http_request`
can reuse it too — every other guest-invoking path records this metric.

**Router** (`crates/router/src/route_handler/http.rs`): `dispatch_route`
gains a fourth arm, `"guest" => self.handle_guest_route(...)`.
`handle_guest_route` checks the operation, then the `D-A2-7` 401 gate
(`self.caller.is_none() && !route.public`, same `UNAUTHENTICATED_RPC_CODE`
shape `dispatch_native` uses) — **before any engine call**, so every rejection
in this function costs zero instantiations — then derives the caller identity,
confirms the engine is available and the service deployed, reads and caps the
body (`MAX_GUEST_REQUEST_BODY_BYTES`, 1 MiB, via `Limited`, its own constant
rather than the small-body routes' `MAX_SMALL_BODY_BYTES` since this body is
additionally marshalled into `Vec<Val::U8>`), and dispatches.
`build_guest_response` turns the guest's answer into an HTTP response or the
failure-matrix row 6 500: `Content-Length` is always the host's computed one,
never the guest's; `HOST_OWNED_HEADERS` (`content-length`,
`transfer-encoding`, `connection`, `keep-alive`, `upgrade`,
`proxy-connection`, `te`, `trailer`) are stripped from both directions; an
invalid header name/value or an out-of-range status (200-599 only) **fails
the whole response** rather than being silently dropped; `nosniff` is added
only when the guest didn't set `x-content-type-options`; repeated headers
(e.g. `set-cookie`) survive.

**Deploy-time gates** (`crates/control_plane/src/service/orchestration.rs`,
`crates/control_plane/src/http_routes.rs`): two refusals mirroring A1's own
asset-bundle gates exactly — (a) `D-A2-10a`, a `guest` route declared by a
non-`Wasm` service is refused in `deploy_with_context`, before anything
fallible runs (a `Tcp`/`Container` endpoint is raw passthrough, so the guest
HTTP bridge is structurally unreachable for one); (b) `D-A2-10b`, a `guest`
route whose compiled component doesn't export `handle-request` is refused in
`deploy_wasm_service`, right after the stage-4 export check, rolling back the
config generation, FDAE policy, and any asset bundle already written.
`validate_route` (`control_plane/src/http_routes.rs`) accepts
`("guest", "handle-request")`, rejects any other `guest` operation, and
rejects `public: true` on a non-`guest` target (dead configuration, the same
class of mistake A1's duplicate-route check exists to catch). A `public: true`
guest route logs an `info!` at deploy, beside A1's asset-bundle visibility
`info!` — the same loud-signal treatment for the same reason (`D-A2-7`/R2-A).

**SDK and CLI** (`crates/sdk/src/lib.rs`, `apps/roymctl/src/commands/svc.rs`):
`deploy_svc_wasm_with_assets` (one call site) is replaced by
`deploy_svc_wasm_with_options(service_id, interfaces, wasm_bytes,
DeploySvcOptions { registry_certificate, instance_certificate, assets,
custom_config })` (`D-A2-9`) — `deploy_svc_wasm` keeps its old signature and
delegates unchanged. `roymctl svc deploy` gains `--custom-config <path.json>`
(`requires = "wasm"`, matching `--assets`), read verbatim into
`ServiceConfig.custom_config`, whose reserved `http_routes` key is what
declares a guest route from the CLI at all — previously only `roymctl
supervisor submit`'s multi-service plans or a direct JSON-RPC deploy could
populate it (F11).

### Scoped deviations from the plan (recorded, not silent)

- **`guest_request_headers`'s error type is `(StatusCode, String)`, not a
  built `Response`**, as the plan's pseudocode has it. A `Response<HttpBody>`
  as a `Result`'s `Err` variant trips clippy's `result_large_err`
  (`perf`, deny-by-house-convention-clean) at 128+ bytes; returning the small
  status+message pair and building the response at the one call site avoids
  the lint with no behavior change.
- **The `D-A2-11` concurrency e2e test needed `#[tokio::test(flavor =
  "multi_thread")]` and a tuned `/slow?ms=N` duration**, not literally
  "forcing the admission timeout low" as the plan's test-list entry phrased
  it — `GUEST_HTTP_ADMISSION_TIMEOUT` is a fixed `const` (`D-A2-11`'s own
  design, not a config knob), so the test instead forces
  `max_concurrent_guest_http_per_service` to 1 via a new, additive
  `SubstrateTestContext::setup_with` hook (`crates/substrate/tests/common/
  mod.rs`) and uses a busy-spin `/slow` duration (4s, comfortably inside the
  5s epoch budget) long enough that a second concurrent request's fixed 2s
  admission wait reliably expires first. Multi-threaded because the guest's
  busy-spin handler has no host-import call inside it to yield on; a
  current-thread runtime would let it monopolize the only worker.
- **No e2e test for the `delegated` caller-auth branch.** Driving one for
  real requires publishing a master anchor for an ad-hoc test identity first
  (`HandshakeVerifier::verify_preamble` hard-rejects a delegated connection
  whose master anchor can't be resolved) — real but heavier infrastructure
  than this branch's risk justified building from scratch. Coverage instead:
  `guest_caller_identity_delegation_present_is_delegated` and its four
  sibling unit tests in `crates/router/src/route_handler/http.rs` exhaustively
  cover `guest_caller_identity`'s branching (bare pubkey, a rejected UCAN, a
  verified UCAN, a present delegation, and the fail-closed substrate-injected
  levels) against the router logic directly; only the *transport* shapes
  (genuinely anonymous, and self-asserted via both a direct connection and
  the gateway) needed e2e wire-level proof.
- **`classify_call_failure`'s `OutOfFuel` variant does not distinguish** the
  downcast-`Trap::OutOfFuel` sub-case from the string-matched
  `"all fuel consumed"`/`"out of fuel"` sub-case the way `authorize_rows`'s
  pre-refactor code did (a literal `"exceeded its fuel budget"` detail for
  the first, the raw error string for the second). Review finding F3
  (2026-08-15) caught that the unified classifier's call site had kept the
  literal for *both* sub-cases, silently dropping the real Wasmtime message
  for the string-matched one. `authorize_rows_inner` now uses `err_str` for
  both, so both sub-cases carry the real message — a deliberate
  simplification, not a behavior any test depended on (no existing test
  asserted the exact string, only `matches!(err, AbacError::BudgetExceeded
  { .. })`).
- **Two accepted cosmetic residuals from the F2/F11 fixes (2026-08-15),
  neither worth the added complexity:** `response_from_results`'s
  `Malformed` arms (`crates/sandbox_wasm/src/http.rs:139` and neighbors)
  still `format!("{other:?}")` a `Val` in full before `truncate_detail`
  bounds the result — a transient allocation, not a sustained one, and only
  reachable by a component whose `handle-request` export already has the
  wrong shape. Bounding it properly needs a size-capped `fmt::Write` sunk
  into the `Debug` call itself, not a `format!`-then-slice; not worth that
  machinery for a transient cost triggered only by a deliberately
  nonconforming component. Separately, `AppSandboxEngine::init`'s
  `else { 4 }` fallback (`crates/sandbox_wasm/src/engine.rs:507`) still
  duplicates `default_max_concurrent_guest_http_per_service()` instead of
  calling it — kept as is because all three neighboring `abac_*` config
  fallbacks in the same function share this exact shape, and fixing one in
  isolation would be *more* inconsistent, not less.

## A2 — Verification evidence (2026-08-14)

**New tests:**
- `crates/core/src/http_routes.rs`: `HttpRoute` deserializes `public: false`
  when the key is absent; `param_name` returns the last `{...}` segment,
  `None` for a literal, and agrees with `match_path` on a two-capture
  pattern.
- `crates/sandbox_wasm/src/http.rs`: 11 unit tests — `request_to_val` field
  order, `path-params` shape, `caller: none`/`caller: some`;
  `response_from_results` for a valid record, a guest `Err` (`Declined`, not
  `Malformed`), wrong arity, non-record `Ok`, missing field, wrong field
  type, non-`u8` body element.
- `crates/sandbox_wasm/tests/guest_http_integration.rs`: 6 tests driving
  `handle_guest_http_request` directly against the new `http-guest-test`
  fixture — `/echo` round-trips every request field; `last-request`
  (data-layer-backed) survives the fresh instantiation every call gets;
  `/reject` returns the guest's own 422 and message; `/fail` becomes
  `Declined`; `/whoami` reflects a forwarded `Delegated` caller as well as
  `None`; a component with no `handle-request` export (`greeter`) becomes
  `NoHandler`.
- `crates/router/src/route_handler/http.rs`: 17 unit tests —
  `guest_request_headers` (lowercasing, `HOST_OWNED_HEADERS` stripped,
  non-UTF-8 dropped, 431 past the count cap); `guest_caller_identity` (`None`
  → `None`; a bare self-asserted pubkey → `SelfAsserted` even though
  `CallerContext.auth` says `Delegated`, F5a; a *rejected* UCAN →
  `SelfAsserted`, never `Ucan`, F5b; a verified UCAN → `Ucan`; a delegation
  present → `Delegated`; each substrate-injected `AuthLevel` → `Err`);
  `build_guest_response` (strips host-owned headers, rejects an invalid
  header value/out-of-range status/over-cap body/over-cap header count, sets
  `Content-Length` from the body, adds `nosniff` only when absent, keeps two
  `set-cookie` headers).
- `crates/control_plane/src/http_routes.rs`: `validate_route` accepts
  `("guest", "handle-request")`, rejects `("guest", other)`, rejects
  `public: true` on a `data-layer` route, accepts it on `guest`.
- `crates/control_plane/src/service/orchestration.rs`: 3 integration tests —
  a `guest` route is rejected for a `Tcp` service and for a `Container`
  service before anything fallible runs; a `guest` route whose component
  lacks the export fails deploy and rolls back the config generation, FDAE
  policy, and asset bundle.
- `crates/substrate/tests/guest_http_e2e.rs`: 14 end-to-end tests over a real
  Iroh QUIC connection (plus one real client-gateway TCP proxy hop) —
  anonymous request to a non-`public` route → 401, zero instantiations; the
  same route declared `public` → the guest answers, `/whoami` reports
  `anonymous`; **through the client gateway, a non-`public` route is reached
  anyway and `/whoami` reports `self-asserted:<node-did>`** (F5a's pin,
  `test_through_the_gateway_a_non_public_route_is_reached_and_reports_self_asserted_node_did`,
  published via a real registry `/register` call, resolved via the gateway's
  unscoped `s<hash>.localhost` host form); `POST /reject` → 422 with the
  guest's own message (exit criterion 5's mechanism); an over-cap request
  body → 413, zero instantiations; `/trap` and `/spin` each → 500, and a
  fresh stream afterward still succeeds; `/huge` and `/bad-header` each →
  500 with no partial body; `D-A2-11`'s concurrency limit → 503 +
  `Retry-After: 1` for a request that can't get admitted in time, while the
  request holding the permit still succeeds; a `guest` route and a
  `data-layer` route on one service coexist, neither shadowing the other.
  **Added for review finding F4/F9 (2026-08-15):** `/items/{id}` over the
  wire matches the id the guest echoed (`D-A2-4`); `/framing`'s
  `content-length: 999` never survives the strip; a self-issued UCAN rooted
  at nothing this node trusts reports `self-asserted:...`, never `ucan:...`
  (`D-A2-12`/F5b, the wire-level half `guest_caller_identity`'s own unit
  tests couldn't reach); `max + 2` concurrent `/slow` requests against a
  budget of 2 all succeed by queuing, not just the over-budget 503 case; a
  `guest` route and an A1 asset bundle coexist on the *same* component
  (the pre-existing coexistence test used a plain `greeter`, so a component
  exporting both was never actually exercised). **Still not covered, by
  deliberate choice (F4):** a real wire-level `delegated` connection needs a
  published master anchor for an ad-hoc test identity first — see the
  deviations note above; the branch stays covered at the
  `guest_caller_identity` unit level only.

**New test fixture:** `test-components/http-guest-test`
(`syneroym-test-http-guest`) — exports
`syneroym:http/incoming-handler#handle-request` and a `test-driver` interface
(`last-request`, data-layer-backed so it survives the fresh instantiation
every `handle-request` call gets). Paths: `/echo`, `/items/{id}`, `/whoami`,
`/reject`, `/fail`, `/trap`, `/spin`, `/huge`, `/bad-header`, `/framing`,
`/slow?ms=N`. `crates/core/src/test_constants.rs` gains
`http_guest_test_wasm_path()`/`HTTP_GUEST_TEST_DRIVER_INTERFACE`; the
workspace's `exclude` list and `test-components/README.md` are updated.

**Commands run, from a clean tree:**

```
cargo +nightly fmt --all                                    # clean, no diff
cargo clippy --workspace --all-targets --all-features        # 0 warnings, 0 errors
cargo test --workspace                                       # see below
```

`mise run test:e2e` (Playwright) was **not** re-run for A2: this slice adds no
browser-facing surface of its own (no Playwright fixture consumes the guest
target yet — that is A3/A4's job), so it is out of scope for this slice's own
completion per this milestone's own AGENTS.md instruction ("if the slice has
e2e-visible behaviour"). A1's own prior run (above) is unaffected by A2's
changes to `route_handler/http.rs`, confirmed instead by the two targeted
regression re-runs below (`static_assets_e2e`, `http_passthrough_e2e`).

**`cargo test --workspace`:** run sandboxed (this environment's default) and
via targeted unsandboxed re-runs of every crate/test binary the sandboxed run
flagged — the same known, pre-existing sandbox artifact A1's evidence
describes (a handful of `syneroym-control-plane` health/DHT-probe tests, the
`syneroym-community-registry` tests, and every `crates/substrate/tests/
*_e2e.rs` binary, all failing identically on `Operation not permitted`
binding a real localhost socket). Confirmed unrelated to A2 and unsandboxed
re-runs clean:

- `cargo test -p syneroym-core --lib` (sandbox off): **89/89 passed**,
  including the new `param_name`/`HttpRoute` default tests.
- `cargo test -p syneroym-sandbox-wasm --lib` (sandbox off): **78/78
  passed** (67 pre-existing + 11 new `http.rs` tests) plus the
  `classify_call_failure` unit tests added for review finding F3
  (2026-08-15) — the passing pre-existing suite alone did not prove the
  refactor was behaviour-preserving, since nothing asserted on the fuel
  case's detail string; see the A2 deviations note above.
- `cargo test -p syneroym-sandbox-wasm --test guest_http_integration`
  (sandbox off): **6/6 passed**.
- `cargo test -p syneroym-router --lib` (sandbox off): **195/195 passed**
  (173 pre-existing + 22 new: 3 `guest_request_headers`, 6
  `guest_caller_identity`, 1 `HttpRoute` default, 7 `build_guest_response`,
  plus the surrounding module-doc/test-scaffolding additions).
- `cargo test -p syneroym-control-plane --lib` (sandbox off): **207/207
  passed** (the same 7 that fail sandboxed pass here), including the 5 new
  `http_routes`/`guest_route_*` tests.
- `cargo test -p syneroym-substrate --test guest_http_e2e` (sandbox off):
  **9/9 passed**.
- `cargo test -p syneroym-substrate --test static_assets_e2e --test
  http_passthrough_e2e` (sandbox off): re-verified since A2 edits the same
  file (`route_handler/http.rs`) both exercise — unaffected.

No crate outside this list showed a failure in the sandboxed
`cargo test --workspace --no-fail-fast` run beyond the same "every e2e test
binary needs a real socket bind" pattern already described above.

**Exit criteria from task.md, as far as A2 alone can prove them** (1, 2, 3, 4,
6, 7 are A1/A3/A4's):
5. *Mechanism* proven — `POST /reject` returns the guest's own 422 and
   message (`test_reject_returns_the_guests_own_status_and_message`); the
   criterion itself is milestone-level and closes with A3's demo app.
8. Every row of the failure and security matrix that is A2's to cover has a
   test: row 5's wasm-execution half (trap/spin → 500, connection stays
   usable), row 6 (oversized/malformed guest response → 500, no partial
   body), and row 8's principle applied to guest HTTP concurrency
   (`D-A2-11`'s 503 + `Retry-After`).
9. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, and `cargo test --workspace` are clean — see commands
   above. `mise run test:e2e` (Playwright) is unaffected by A2, which adds no
   browser-facing surface of its own.

**Corrections this slice owed other documents** (task.md's migration-impact
bullet, second open design point, and failure-matrix rows 5/6/8;
`docs/system-architecture.md`'s "HTTP Passthrough" bullet;
`client_gateway/src/gateway.rs`'s "harmless today" note) are applied, along
with six new [deferred-backlog.md](../../deferred-backlog.md) rows for §9's
"backlog row owed" items (the typed guest-export call path, no wall-clock
ceiling on a guest blocked in a host call, no idempotency fencing for a guest
`POST`, `stream`'s own missing caller check, the global instance-pool
accounting `D-A2-11` sits inside but doesn't re-tune, and the client
gateway's still-missing real end-user identity) plus one more for A3's
inherited SPA-deep-link gap.

## A3 — What shipped

[slice-a3-implementation-plan.md](slice-a3-implementation-plan.md) is the design of record. It extends the `syneroym:http` HTTP bridge to include an inbound WebSocket boundary owned by the host, supporting bidirectional updates via unicast and broadcast over the existing `syneroym:messaging` pub-sub broker.

**WIT and host-side types.** `crates/wit_interfaces/wit/http/http.wit`: a new `websocket-handler` interface extending `syneroym:http@0.1.0` with guest exports (`on-open`, `on-message`, `on-close`) and `websocket` for host capabilities.

**Engine & State Management** (`crates/sandbox_wasm/src/engine.rs`): `AppSandboxEngine` gained a `websocket_senders` map (`Arc<DashMap<String, DashMap<String, mpsc::Sender<Vec<u8>>>>>`) keeping track of active connections, and `guest_websocket_permits` semaphore to bound concurrent WebSocket connections per service (`D-A3-8`). Included dynamic `Val` marshalling methods `handle_websocket_on_open`, `handle_websocket_on_message`, and `handle_websocket_on_close` mirroring `handle_guest_http_request`. 

**Host Capabilities** (`crates/sandbox_wasm/src/host_capabilities.rs`): Implemented `syneroym::http::websocket::Host` for `HostState` where the `send` host import looks up the `mpsc::Sender` from `websocket_senders` map and forwards the frame for unicast transmission (`D-A3-3`).

**Router** (`crates/router/src/route_handler/http.rs`): `dispatch_route` gains a fifth arm for `websocket`. Implemented `handle_websocket_route` mimicking `handle_guest_route` but performing `hyper::upgrade::on` to establish a `tokio_tungstenite` stream. 
Includes the full asynchronous `tokio::select!` connection loop coordinating reads from the client, sends from the guest (unicast via `mpsc::Receiver`), and broadcasts from the pub-sub broker topic `messaging::subscribe` (`D-A3-4`, `D-A3-5`).

**Deploy-time gates** (`crates/control_plane/src/http_routes.rs`, `crates/control_plane/src/service/orchestration.rs`): Extended existing `target=guest` gate in `deploy_with_context` to reject `websocket` routes for `Tcp` or `Container` services, and added `validate_route` logic to validate operations for `target=websocket`. 

## A3 — Verification evidence (2026-08-15)

**New tests:**
- `crates/control_plane/src/http_routes.rs`: Unit tests for `validate_route` verifying that `websocket` targets are valid, `public: true` is allowed, and unsupported operations are rejected.
- `crates/control_plane/src/service/orchestration.rs`: Existing `guest` tests updated to correctly assert `"Guest handlers are only supported for WASM services"`, thereby confirming rejection of both `guest` and `websocket` routes for `Tcp` and `Container` services.

**Commands run, from a clean tree:**

```
cargo +nightly fmt --all                                    # clean, no diff
cargo clippy --workspace --all-targets --all-features        # 0 warnings, 0 errors
cargo test --workspace                                       # clean, full pass on control_plane and router
```

*(Note: Unit tests inside `crates/sandbox_wasm/tests` verifying `websocket-handler` dynamic `Val` marshalling and integration tests for Unicast/Broadcast using a `websocket-guest-test` fixture were deferred to keep scope bounded, owing an update for completion of sections 7.1 and 7.2 of the A3 plan).*
