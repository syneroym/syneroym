# M06A Slice A1 — Blob-Backed Static Serving: Implementation Plan

> **Milestone:** [task.md](task.md) · **Slice:** A1 · **Status:** Planned, not
> started. **Revision 2** (2026-08-14), after review.
>
> Decision ids are `D-A1-n`. Milestone-level decisions (`D-06A-n`) live in
> [task.md](task.md) and are inputs here.

---

## §0 Review response (revision 2)

Every finding from the 2026-08-14 review was verified against `main` before
disposition. All were accepted; three are accepted with a narrower or different
fix than the review implied, argued below.

| # | Finding | Disposition |
|---|---|---|
| 1 | SPA fallback swallows `/api/*`; breaks A1/A2 independence | **Accepted.** Fallback removed from A1 entirely (see also 7). Matching is exact-path, `GET`/`HEAD` only, with deploy-time collision detection against declared routes — `D-A1-4` |
| 2 | 64 MiB cap cannot travel; `MAX_FRAME_SIZE` is 16 MiB | **Accepted.** Verified `crates/rpc/src/framing.rs:20`. Cap cut to 4 MiB and the budget is *shared with the component binary* — a point the review understates. `D-A1-5` |
| 3 | `artifact-source::url` is a dead branch | **Accepted.** Verified: `orchestration.rs:1189-1213`'s `else if` requires *not* `http(s)://`, so a URL falls through untouched. Doc comment corrected; reviving it is out of scope and now a backlog row |
| 4 | Manifest blob written, never read; "persist" is really "cache" | **Accepted, different fix.** No boot-time deploy-state restore exists *anywhere* — nothing reads `hosted_apps_dir` at boot and it holds only the registry certificate. Building one is not A1. Taken as the review's own second option: explicit decision + backlog row — plus a fix for the orphan-blob half, which is **not** merely mirroring `http_routes` (`D-A1-2`, `D-A1-9`) |
| 5 | Redeploy leaks the previous bundle | **Accepted.** `D-A1-9`, call site 6 |
| 6 | `public: bool` is what ADR-0018 explicitly rejects | **Accepted, refined.** Verified the rejection by name at ADR-0018's *Alternatives considered*. Adopting its `visibility` enum — but on `asset-bundle`, not `service-config`, because ADR-0018 has claimed that field name for a different question. Argued in `D-A1-1` |
| 7 | SPA fallback is unsanctioned scope | **Accepted.** Removed; moved to A3 where a consumer exists |
| 8 | §6 overclaims coverage | **Accepted.** Claim corrected, two mis-mapped tests fixed |
| 9 | `supervisor.rs` inlining missed; two path-resolution rules | **Accepted.** Call site 4; `D-A1-1` now states one rule explicitly |
| 10 | Wrong file for the control-plane field | **Accepted.** Verified `crates/control_plane/src/service.rs:144`, `init` at `:171` |
| 11 | `mapper.rs:300` is a `ContainerManifest` | **Accepted.** Verified |
| 12 | `mime_guess` belongs to control-plane | **Accepted.** §4.1 computes content type at unpack |
| 13 | `new_coordinator` also builds `RouteHandlerInner` | **Accepted.** Call site 11 |
| S1-S6 | Non-UTF-8 paths, Cache-Control inversion, signature mismatch, HEAD, `/blobs/` shadowing, counter disagreement, blob quotas | **All accepted.** §2.4, §4.1, §4.3, `D-A1-7`, `D-A1-10` |

The review also confirmed two things this plan asserts, which stay: F3's
native-dispatch argument, and that anonymous browser connections reach the
handler with `caller = None` rather than being rejected.

### Revision 3 (2026-08-14, second review)

All six accepted; two were defects the revision-2 fixes introduced. Two further
consequences follow that the review did not name — R3-B and R3-C below.

| # | Finding | Disposition |
|---|---|---|
| 1 | `GET /` resolves to nothing — removing the fallback removed directory-index behaviour too | **Accepted.** A directory index is not history fallback; task.md's reference scenario step 4 requires it. `D-A1-11` |
| 2 | `D-A1-9` deletes blobs the new generation still references | **Accepted — correctness bug.** Verified `delete_blob` is an unconditional single-object delete with no refcount (`object_store_impl.rs:295-307`). `D-A1-9` now deletes `old_hashes − new_hashes` |
| 3 | Collision check misses parameterised routes | **Accepted.** `match_path` matches segment count then captures `{...}`, so set membership is insufficient |
| 4 | 4 MiB is already over the limit; the arithmetic settles P1 | **Accepted.** Verified `additional_derives: [serde::Serialize, …]` (`wit_interfaces/src/control_plane.rs:9`) and `serde_json::to_value` (`sdk/src/lib.rs:687`). Average is exactly (10·2 + 90·3 + 156·4)/256 = **3.57**. Cap cut to 2 MiB; see R3-C |
| 5 | Leak window between unpack and registry insert | **Accepted.** Verified the native-capability failure path calls `self.undeploy(..)` at `orchestration.rs:1749` |
| 6 | `private`-by-default fails silently for the operator | **Accepted.** `D-A1-1`, deploy-time `info!` |

**R3-A — `match_path` is private and in the wrong crate.** Finding 3's fix needs
`match_path`, which is `fn match_path` (not `pub`) at
`crates/router/src/route_handler/http.rs:247`, while the collision check runs in
`syneroym-control-plane` at deploy. It moves to `syneroym_core::http_routes`
beside `HttpRoute` — the same reason that type lives in core rather than the
router.

**R3-B — rollback has the mirror image of finding 2's bug.** Finding 2 is about
deleting *forward*: the old bundle's blobs must skip hashes the new manifest
references. The mirror case is deleting *backward*: on a failure path the new
generation is abandoned while the **old manifest stays live**, so rollback must
skip hashes present in the *old* manifest or it deletes blobs the still-serving
previous generation points at. Same content-addressing, opposite direction.
Both directions are now one helper — `D-A1-9`.

**R3-C — a fixed per-bundle cap is the wrong shape, though a smaller one still
helps.** The real constraint is
`encoded(component) + encoded(bundle) + envelope < MAX_FRAME_SIZE`. Any fixed
bundle cap is either too loose beside a large component or needlessly tight
beside a small one. The client-side **combined** check becomes authoritative;
the fixed cap stays only as a cheap early guard.

---

## §1 Findings from reading the tree

### F1 — A1 must add WIT surface

[task.md](task.md)'s *Migration impact* claim that only A2 might change WIT is
stale. Nothing existing carries a binary bundle for a WASM service:
`artifact-source` is on `wasm-manifest`/`container-manifest` only;
`document-source::inline` is `string`; `container-volume-file` is container-only
and text-only. Correction owed — §8.

### F2 — the deploy path's real size ceiling, and who shares it

`MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024` (`crates/rpc/src/framing.rs:20`), and
the SDK sends a deploy as one JSON-RPC frame.

**The budget is shared.** The same frame carries the component binary *and*
the asset bundle. On the supervisor `submit` path both are hex-encoded
(`INLINE_ARTIFACT_PREFIX`, `crates/sdk/src/mapper.rs:30`), doubling each. So the
usable asset budget is not ~7.9 MiB but 16 MiB minus twice the component size
minus envelope — for a 2 MiB component, under 6 MiB.

**Settled in revision 3 — it is worse than hex, and it is arithmetic, not a
measurement.** The WIT bindings derive plain serde
(`additional_derives: [serde::Serialize, serde::Deserialize]`,
`crates/wit_interfaces/src/control_plane.rs:9`), so `list<u8>` is a `Vec<u8>`,
and the deploy manifest goes through `serde_json::to_value`
(`crates/sdk/src/lib.rs:687`) — a JSON **array of integers**, not a string.
Cost per artifact byte, including the separating comma: 2 chars for 0-9, 3 for
10-99, 4 for 100-255, so for the uniformly-distributed bytes of a gzip archive
the mean is exactly:

```
(10·2 + 90·3 + 156·4) / 256  =  914 / 256  =  3.57 bytes of JSON per byte
```

Against a 16 MiB frame shared with the component:

| Component | Encoded | Bundle budget left | Max compressed bundle |
|---|---|---|---|
| 1 MiB | ~3.6 MiB | ~12.4 MiB | ~3.4 MiB |
| 2 MiB | ~7.1 MiB | ~8.9 MiB | ~2.4 MiB |
| 4 MiB | ~14.3 MiB | ~1.7 MiB | ~0.4 MiB |

Revision 2's 4 MiB cap was therefore already unshippable: 4 MiB of assets alone
encodes to ~14.3 MiB and leaves ~1.7 MiB for everything else. The supervisor
`submit` path is cheaper on its first hop (hex, 2×) but pays the same 3.57× on
the supervisor → substrate hop, so it does not escape this.

P1 keeps the measurement, demoted from a gate to a **confirmation** of the
arithmetic above.

### F3 — the blob path is `NativeService`-shaped, so the sandbox is never touched

`dispatch_json_rpc_unfenced` (`crates/router/src/route_handler/dispatch.rs:191`)
branches on `(&pipeline.adaptation, &pipeline.service)`:
`(AdaptationStage::None, ServiceStage::NativeService)` → host;
`(JsonRpcToWasm, WasmComponent)` → `app_sandbox_engine`. An
`http://http-native|<service_id>` connection resolves `pipeline.service` from a
native-capability interface, so the synthetic `blob-store` preamble
`handle_blob_get` already uses takes the first arm. `D-06A-1` holds by
construction. *(Confirmed independently in review.)*

### F4 — per-chunk JSON-RPC round trip is heavy, and there is no `BlobProvider` on the router

`handle_blob_get` costs one `open-download` plus N × `read-chunk`, each fully
encoded and decoded. `RouteHandlerInner`
(`crates/router/src/route_handler.rs:88`) holds `key_store` (:126) and
`storage_provider` (:127) but no `BlobProvider`. See `D-A1-6` and §7.1.

### F5 — no instantiation counter exists

`crates/sandbox_wasm/src/engine.rs` has
`histogram!("substrate.wasm.instantiation_ms")` (:1023) and
`gauge!("substrate.wasm.active_instances")` (:786/:792). Exit criterion 3 needs
a count. `D-A1-7`.

### F6 — dependency states differ

`mime_guess` new to the workspace (2.0.5 exists only in the excluded
`miniapp-demo1-web`); `tar` new outright; `flate2` present at `1.1.9` in
`crates/coordinator_webrtc/Cargo.toml:40` but not a workspace dependency.

### F7 — the traversal guard exists, with one caveat

`syneroym_core::deploy_docs::reject_relative_escape(&str, &str)` (:119) rejects
`..`, root, drive prefixes, and empty. Two caveats the review caught: it takes
`&str` while `tar::Entry::path()` yields `Cow<Path>` (non-UTF-8 names must be
rejected first), and its message is hardcoded "must be a relative path inside
the volume" (`deploy_docs.rs:129-133`), which reads wrong for an archive entry.

### F8 — ADR-0018 has already answered the visibility question

ADR-0018 adds `visibility: option<visibility>` with `enum visibility { public,
internal, private }` to `service-config` (WIT) and `ServiceConfig` (Rust),
default private (:81, :104-107, :114), and **rejects `public: bool` by name** in
*Alternatives considered* (:268). `D-A1-1`.

### F9 — the blob hash is over plaintext

`plaintext_hasher` is updated before encryption and returned by `finish()`
(`crates/data_blob/src/object_store_impl.rs:387`, :401). At-rest encryption is
orthogonal to ETag correctness. `D-A1-3`.

### F10 — nothing restores deploy state at boot

`hosted_apps_dir` is written only with the registry certificate
(`orchestration.rs:1514`) and read nowhere at startup; `runtime.rs` has no
restore/rehydrate path. `http_routes` is created empty each boot
(`runtime.rs:991`) and is never repopulated. This is the pre-existing shape,
consistent with several accepted backlog rows (signed topology documents,
registry aliases, the KEK). `D-A1-2`.

---

## §2 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A1-1** | New `asset-bundle` record on `service-config`, carrying `archive: artifact-source`, `hash: option<string>`, and **`visibility: option<visibility>`** reusing ADR-0018's enum, default `private`. No `spa-fallback`. | F8: a bool is rejected by name. The enum goes on `asset-bundle`, **not** `service-config`, because ADR-0018 claims that field for a different question — whether the *endpoint record* is published — and A1 asks whether *these bytes* are readable by an unauthenticated caller. Same type, distinct field, no collision and no second shape. `public` serves assets to anyone; `internal` and `private` both refuse (A1 implements no middle tier). **Deploy emits one `info!` naming the entry count and the resolved visibility** — `D-A1-8`'s silence is deliberately a caller-side property, and without this an author who forgets `--asset-visibility public` gets 404s with no signal anywhere (revision-3 finding 6). |
| **D-A1-2** | The registry is a **cache, not persistence**. The manifest is still stored as its own blob so a future loader has something to read, but A1 adds no boot-time loader. | F10: no deploy-state restore exists anywhere to hang one on. Building one is a milestone-sized change. Stated as a decision, not buried — with `D-A1-9` covering the part that is genuinely new, and a backlog row (§8). |
| **D-A1-3** | Assets use the service DEK like every other blob. | F9. One storage path; `storage.encryption = false` already covers operators wanting plaintext globally. |
| **D-A1-4** | Static matching is **exact-path (plus `D-A1-11`'s directory index), `GET`/`HEAD` only**, placed before `resolve_route`. A deploy is **refused** when any asset path is matched by a declared **`GET`/`HEAD`** route *pattern* — evaluated with `match_path`, not set membership, and filtered by method first. | Revision-3 finding 3: `match_path` matches segment count then lets `{param}` capture anything, so `GET /docs/{slug}` and an asset at `/docs/intro.html` collide despite no literal equality. Requires R3-A's move of `match_path` into core. The method filter (revision-4 finding 4) avoids a false rejection: assets never answer `POST`, so a `POST /docs/{slug}` has no request-time conflict to prevent. |
| **D-A1-5** | The **authoritative** check is client-side and combined: `encoded(component) + encoded(bundle) + envelope` must fit `MAX_FRAME_SIZE`, at 3.57 bytes per artifact byte, with an error naming both encoded sizes and the frame limit. `MAX_ASSET_BUNDLE_BYTES = 2 MiB` is retained as a cheap early guard, plus `MAX_ASSET_UNPACKED_BYTES = 64 MiB` and `MAX_ASSET_FILE_COUNT = 10_000`. | F2 as settled in revision 3, and R3-C: a fixed bundle cap is either too loose beside a big component or needlessly tight beside a small one, so the combined check is the real rule. 2 MiB fits alongside a 2 MiB component with headroom. Unpacked cap guards decompression bombs independently of both. |
| **D-A1-11** | A request path ending in `/` resolves to `<path>index.html`, exactly. Nothing else falls back. | Revision-3 finding 1. task.md's reference scenario step 4 requires `GET /` → index.html, and a browser opening the gateway hostname asks for exactly that. This is a **directory index**, not SPA history fallback: it fires only on a trailing slash, so `/api/comments` is untouched and A1/A2 independence survives. `/some/route` → index.html remains A3's problem. |
| **D-A1-6** | Reuse the existing `dispatch_native` blob path; do not add a `BlobProvider` to `RouteHandlerInner`. | F4 is real but fixing it blind is premature. Self-contained follow-up; backlog row owed regardless (§7.1). |
| **D-A1-7** | Add **both**: `metrics::counter!("substrate.wasm.instantiations_total")` and an `AtomicU64` on `AppSandboxEngine` with a getter. Tests assert on a **delta measured around the request**, never an absolute. | F5. The review is right that the two were inconsistent, and right that deploy-time lifecycle hooks instantiate the component — so an absolute assertion is wrong regardless of mechanism. |
| **D-A1-8** | A miss returns **404, never 403**; a non-`public` bundle is byte-identical to no bundle. | Failure-matrix row 7. |
| **D-A1-9** | One helper, `delete_hashes(service_id, remove: &BTreeSet<String>, keep: &BTreeSet<String>)`, used in **both** directions and always subtracting `keep`. **Forward** (successful redeploy): remove the old manifest's hashes, keep the new manifest's. **Backward** (any failure between unpack and registry insert): remove what this deploy wrote, keep the still-live old manifest's. The written-hash set lives in a scope guard reachable from every early return, not only from inside `unpack_asset_bundle`. | Revision-3 findings 2 and 5, plus R3-B. Blobs are content-addressed with one object per `(service_id, hash)` and no refcount (`object_store_impl.rs:295-307`), so an unchanged file has the *same* hash in both generations — deleting either manifest's hashes wholesale destroys live data. With `D-A1-2` there is no boot-time recovery, so both a leak and a wrongful delete are permanent. |
| **D-A1-10** | Unpack surfaces `BlobError` quota failures as deploy failures and rolls back. | Blob quotas (`max_blob_bytes` default 100 MiB, optional `max_service_total_bytes`, `crates/core/src/config.rs:237-250`) are enforced by `UploadSession::write` independently of `D-A1-5`. A bundle can pass A1's caps and still fail a quota mid-unpack. |

---

## §3 Exact type and signature changes

### 3.1 WIT — `crates/wit_interfaces/wit/control-plane/control-plane.wit`

```wit
    /// Endpoint-record visibility (ADR-0018). Defined here by A1 because A1
    /// is the first consumer; ADR-0018 adds the `service-config` field that
    /// uses it for record publication, a different question from asset
    /// readability below.
    enum visibility { public, internal, private }

    /// A bundle of static assets served straight from blob storage without
    /// instantiating the component. Unpacked at deploy; the router resolves
    /// an exact request path to a blob hash and streams it.
    record asset-bundle {
        /// gzip-compressed tar archive of the site root.
        archive: artifact-source,
        /// Hex sha256 of the *compressed* archive, verified before
        /// unpacking. Absent skips the check.
        hash: option<string>,
        /// `public` serves these bytes to any caller with no signature and
        /// no delegation. `internal` and `private` both refuse, identically
        /// to having no bundle. Absent means `private`.
        visibility: option<visibility>,
    }
```

One field on the existing `record service-config`:

```wit
        /// Static assets served directly from blob storage (M06A A1).
        assets: option<asset-bundle>,
```

### 3.2 Rust — `crates/app_orchestration/src/models.rs`

```rust
/// Endpoint/asset visibility (ADR-0018). Defaults to the most private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Internal,
    #[default]
    Private,
}

/// Author-side declaration of a static asset bundle.
///
/// `archive` resolves **against the manifest's own directory** when it is a
/// bare relative path -- deliberately *not* `ServiceConfig::source`'s rule,
/// which resolves against the client process's cwd. Two rules already exist
/// in the tree (`mapper` uses cwd, `roymctl supervisor submit` uses
/// `manifest_dir`); a new field picks the one that is not surprising and
/// says so, rather than claiming a single rule exists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetBundle {
    pub archive: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default)]
    pub visibility: Visibility,
}
```

On `ServiceConfig` (`models.rs:475`), after `health_check`:

```rust
    /// Static assets served directly from blob storage (M06A A1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<AssetBundle>,
```

### 3.3 New module — `crates/core/src/asset_manifest.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetEntry {
    /// Blob content hash (hex sha256 of plaintext) -- also the ETag value.
    pub hash: String,
    pub len: u64,
    /// Resolved at unpack time from the path extension.
    pub content_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetManifest {
    /// Request path, normalised to a single leading '/', -> entry.
    pub entries: BTreeMap<String, AssetEntry>,
}

#[derive(Debug, Clone)]
pub struct ServiceAssets {
    pub manifest: Arc<AssetManifest>,
    /// Only `Public` serves; every other value refuses (D-A1-1, D-A1-8).
    pub visibility: Visibility,
    /// Retained so `deploy`'s next generation and `undeploy` can delete the
    /// manifest blob itself, not only the entries (D-A1-9).
    pub manifest_hash: String,
}

/// Shared, keyed-by-`service_id` static asset table. **A cache, not
/// persistence** (D-A1-2): created empty each boot, populated by
/// `deploy()`, cleared by `undeploy()`. Same shape and lifecycle as
/// `HttpRouteRegistry`.
pub type AssetRegistry = Arc<DashMap<String, ServiceAssets>>;
```

### 3.4 Caps — `crates/core/src/deploy_docs.rs`

```rust
/// Cheap early guard on a compressed asset archive. **Not** the real
/// limit -- that is `D-A1-5`'s combined client-side check, because the
/// 16 MiB frame is shared with the component binary and both expand
/// ~3.57x as JSON integer arrays. 2 MiB is what fits beside a component
/// of realistic size; a bundle under it can still be refused by the
/// combined check.
pub const MAX_ASSET_BUNDLE_BYTES: u64 = 2 * 1024 * 1024;

/// The archive's unpacked total. A compressed-only cap is a
/// decompression-bomb lever.
pub const MAX_ASSET_UNPACKED_BYTES: u64 = 64 * 1024 * 1024;

pub const MAX_ASSET_FILE_COUNT: usize = 10_000;

/// Like `reject_relative_escape`, but for an archive entry: rejects a
/// non-UTF-8 name before the lexical check, and names the archive rather
/// than "the volume" in its errors (F7).
pub fn reject_archive_entry_path(path: &Path, field_name: &str) -> Result<String, String>;
```

### 3.4a `match_path` moves to core (R3-A)

`fn match_path(pattern: &str, path: &str) -> Option<Option<String>>` is private
and lives at `crates/router/src/route_handler/http.rs:247`, but `D-A1-4`'s
collision check runs in `syneroym-control-plane` at deploy. Move it to
`syneroym_core::http_routes` beside `HttpRoute`, `pub`, and have the router call
it there:

```rust
/// Matches a declared route pattern against a concrete path. Segment count
/// first, then per-segment: `{name}` captures anything, everything else must
/// match literally. `Some(None)` is a match with no capture, `Some(Some(v))`
/// a match capturing `v`, `None` no match.
pub fn match_path(pattern: &str, path: &str) -> Option<Option<String>>;
```

Move its four unit tests (`http.rs:1094-1111`) with it. Same reason `HttpRoute`
itself lives in core: two crates need one definition of what a route path means.

### 3.5 `RouteHandlerInner` — `crates/router/src/route_handler.rs:88`

Beside `http_routes` (:117):

```rust
    /// Static asset manifests, per service. Same `Arc` and same producer as
    /// `http_routes` above; a cache, not persistence (M06A D-A1-2).
    pub assets: AssetRegistry,
```

Not `Option`, unlike `key_store`/`storage_provider` — `RouteHandler::new_coordinator`
passes an empty registry, so coordinator mode serves no assets and needs no
branch.

### 3.6 `ControlPlaneService` — `crates/control_plane/src/service.rs`

Field beside `http_routes` (:144); `init` (:171) takes the `Arc`.

### 3.7 New deploy-side module — `crates/control_plane/src/assets.rs`

```rust
/// Unpacks into blobs and returns the manifest.
///
/// **Writes every hash it stores into `written` as it goes, including on
/// the error path.** `unpack_asset_bundle` therefore performs no deletion
/// of its own: the caller owns `written` and its scope guard, which is
/// also the only place that can see the still-live old manifest and so the
/// only place that can compute `keep` correctly (R3-B). This split keeps
/// both halves unit-testable -- unpack against "what did it write", and
/// `delete_hashes` as a pure set difference.
pub async fn unpack_asset_bundle(
    service_id: &str,
    archive: &[u8],
    declared_hash: Option<&str>,
    /// Route *patterns*, not literal paths -- checked with `match_path`
    /// per route, and only for routes answering GET/HEAD (D-A1-4, R3-A).
    declared_routes: &[HttpRoute],
    blob: &Arc<dyn BlobProvider>,
    dek: Option<Zeroizing<[u8; 32]>>,
    written: &mut BTreeSet<String>,
) -> Result<AssetManifest, String>;

/// The one deletion helper, used in both directions (D-A1-9). Deletes
/// `remove - keep`, never a hash `keep` contains. Forward: remove the old
/// manifest's hashes, keep the new manifest's. Backward: remove what this
/// deploy wrote, keep the still-live old manifest's.
pub async fn delete_hashes(
    service_id: &str,
    remove: &BTreeSet<String>,
    keep: &BTreeSet<String>,
    blob: &Arc<dyn BlobProvider>,
) -> Result<(), String>;

pub async fn store_manifest(
    service_id: &str,
    manifest: &AssetManifest,
    blob: &Arc<dyn BlobProvider>,
    dek: Option<Zeroizing<[u8; 32]>>,
) -> Result<String, String>;

/// Every hash a manifest references, including the manifest blob's own
/// when given -- the `remove` or `keep` argument to `delete_hashes`.
pub fn hashes_of(manifest: &AssetManifest, manifest_hash: Option<&str>) -> BTreeSet<String>;
```

### 3.8 Serving — `crates/router/src/route_handler/http.rs`

```rust
/// Exact-path lookup, plus D-A1-11's one rewrite: a `path` ending in `/`
/// resolves to `<path>index.html`. **This function owns the rewrite** --
/// callers pass the raw request path and do no normalisation of their own,
/// so there is exactly one place the rule lives.
///
/// `None` when the service has no bundle, its visibility is not `Public`,
/// or no entry matches -- indistinguishable by D-A1-8.
fn resolve_asset(&self, path: &str) -> Option<AssetEntry>;

/// Serves one asset. `Ok(None)` means "not an asset" and the caller falls
/// through to route resolution unchanged.
async fn try_handle_asset(
    &self,
    method: &Method,
    path: &str,
    req: &Request<Incoming>,
) -> Result<Option<Response<HttpBody>>>;
```

### 3.9 Counter — `crates/sandbox_wasm/src/engine.rs`

In `build_store_and_instantiate` (:923), beside the histogram at :1023:

```rust
metrics::counter!("substrate.wasm.instantiations_total").increment(1);
self.instantiations.fetch_add(1, Ordering::Relaxed);   // pub fn instantiations() -> u64
```

---

## §4 Call sites

`ServiceConfig` gains a `#[serde(default)]` field, so only struct literals break.

| # | File | Change |
|---|---|---|
| 1 | `wit/control-plane/control-plane.wit` | `visibility` enum, `asset-bundle`, `service-config.assets` |
| 2 | `app_orchestration/src/models.rs` | `Visibility`, `AssetBundle`, `ServiceConfig::assets` |
| 3 | `sdk/src/mapper.rs` | Factor lines 194-220's three-branch resolution into `fn resolve_artifact_source(source: &str, what: &str) -> anyhow::Result<ArtifactSource>`; call from the Wasm arm and the new assets mapping. Add `assets` to the WIT `service-config` literal **ending at :191** (not :300, which is a `ContainerManifest`) |
| 4 | `apps/roymctl/src/commands/supervisor.rs:231-244` | Inlines only `config.source` today. Must also inline `assets.archive`, resolved against `manifest_dir` per `D-A1-1`, or a remote `submit` reaches a substrate that cannot read the path |
| 5 | `control_plane/src/service/orchestration.rs` ~1534-1538 | After `parse_http_routes`: resolve the archive, unpack, store the manifest. Needs the service DEK — same `load_service_dek` call `resolve_blob_dek` makes. Pass the parsed `&[HttpRoute]` (patterns, not paths) for `D-A1-4`'s check. **Arm the scope guard here** holding every hash written, so any early return between this point and call site 6 runs `D-A1-9`'s backward delete |
| 5a | `control_plane/src/service/orchestration.rs:1749` and every sibling failure path that calls `self.undeploy(..)` between 1538 and 1789 | These are the leak window (revision-3 finding 5): at that moment the registry still holds the **previous** generation, so `undeploy` deletes the old bundle while the newly written blobs orphan. The guard from call site 5 must fire on these paths, keeping the old manifest's hashes |
| 6 | `control_plane/src/service/orchestration.rs` ~1789-1792 | Insert the new `ServiceAssets`, then forward-delete: `delete_hashes(remove = old_manifest.hashes, keep = new_manifest.hashes)`. **Not** a wholesale delete of the old bundle — unchanged files share hashes across generations (revision-3 finding 2). Disarm the guard only after this succeeds. Deploy overwrites in place (`:1275`); there is no implicit undeploy |
| 7 | `control_plane/src/service/orchestration.rs:2270` | Beside `http_routes.remove`: remove from `assets` **and** `delete_hashes(remove = manifest.hashes ∪ {manifest_hash}, keep = ∅)` — nothing survives an undeploy, so there is nothing to keep |
| 8 | `control_plane/src/service.rs:144`, `init` at `:171` | New field; constructor takes the `Arc` |
| 9 | `router/src/route_handler.rs:88` | New `assets` field |
| 10 | `substrate/src/runtime.rs:991`, `ControlPlaneService::init(` call opening at `:1000` | Create one `AssetRegistry` `Arc` beside `http_routes`; clone into both `ControlPlaneService::init` and `RouteHandlerInner` |
| 11 | `RouteHandler::new_coordinator` | Empty `AssetRegistry` — coordinator mode serves no assets |
| 12 | `router/src/route_handler/http.rs:380` `try_handle_http_request` | Insert `try_handle_asset` **between** the `blob_hash_from_path` branch (:384-389) and `self.resolve_route(..)` (:391-393). Note the `/blobs/<hex>` branch at :383 shadows any site path under `/blobs/` — documented as a reserved prefix, rejected at deploy |
| 12a | `router/src/route_handler/http.rs:247` → `core/src/http_routes.rs` | Move `match_path` to core as `pub`, with its four tests (`http.rs:1094-1111`); router calls it from core (R3-A). Needed because `D-A1-4`'s check runs in control-plane |
| 13 | `core/src/lib.rs` | `pub mod asset_manifest;` |
| 14 | `core/src/deploy_docs.rs` | Three consts + `reject_archive_entry_path` |
| 15 | `sandbox_wasm/src/engine.rs:1023` | Counter + `AtomicU64` field and getter |
| 16 | `Cargo.toml`, `crates/control_plane/Cargo.toml` | `mime_guess = "2.0.5"` and `tar` → **control-plane**, not the router (content type is computed at unpack). Promote `flate2` (`coordinator_webrtc/Cargo.toml:40`, `1.1.9`) to `[workspace.dependencies]`; both consumers use `flate2.workspace = true` |
| 17 | `apps/roymctl/src/commands/svc.rs` | Optional `--assets <path>` / `--asset-visibility`; any `ServiceConfig` literal gains the field |

`cargo check --workspace` after item 2 enumerates the remainder. Do not
hand-search.

---

## §5 Pseudo-code

### 5.1 Unpack

```
if archive.len() > MAX_ASSET_BUNDLE_BYTES -> Err
verify declared_hash against sha256(archive) if present

# `written` is the caller's `&mut BTreeSet<String>` (§3.7) -- NOT a local.
# A local would be dropped on the error path, which is the exact failure
# this split exists to prevent (D-A1-9, D-A1-10, R3-B).
total   = 0
entries = BTreeMap::new()

for entry in tar::Archive::new(GzDecoder::new(archive)).entries():
    if !entry.header().entry_type().is_file(): continue   # dirs, symlinks skipped

    key = reject_archive_entry_path(entry.path(), "assets archive entry")?
          # rejects non-UTF-8 first, then '..'/root/prefix/empty (F7)
    key = normalise(key)                                  # strip "./", one leading '/'

    if key starts with "/blobs/" -> Err                   # reserved (call site 12)

    # D-A1-4: patterns, not literals. `GET /docs/{slug}` collides with an
    # asset at `/docs/intro.html` even though neither string equals the
    # other. Method-filtered: assets only ever answer GET/HEAD, so a
    # `POST /docs/{slug}` cannot conflict at request time and refusing it
    # would be a false rejection of a valid deploy.
    for r in declared_routes where r.method is GET or HEAD:
        if core::http_routes::match_path(&r.path, key).is_some() -> Err
    if entries.len() + 1 > MAX_ASSET_FILE_COUNT -> Err

    read entry in chunks, aborting the moment
        total + read > MAX_ASSET_UNPACKED_BYTES -> Err    # inside the loop, not after
    total += len

    hash = blob.put_blob(service_id, bytes, dek.clone()).await?
             # D-A1-10 quota failure lands here. No rollback in this fn --
             # `written` has been accumulating as we go, and the caller's
             # guard does the deleting, because only it can see the old
             # manifest needed for `keep` (R3-B).
    written.insert(hash)     # BEFORE any further fallible step

    entries.insert(key, AssetEntry {
        hash, len,
        content_type: mime_guess::from_path(&key).first_or_octet_stream().to_string(),
    })

Ok(AssetManifest { entries })
```

Two things that must not be got wrong: the unpacked-size check is *inside* the
read loop (checking after means the bomb already expanded), and non-regular
entries are **skipped**, not rejected — so an ordinary `tar czf` of a directory
is accepted, and a symlink is never followed, which is why the lexical guard
suffices and no canonicalisation is needed.

### 5.2 Resolve and serve

```
try_handle_asset(method, path, req) -> Result<Option<Response>>:
    if method != GET && method != HEAD: return Ok(None)     # finding 1

    a = assets.get(service_id) else return Ok(None)
    if a.visibility != Public: return Ok(None)              # D-A1-8

    entry = resolve_asset(path) else return Ok(None)
    # resolve_asset owns normalisation and D-A1-11's trailing-slash rewrite
    # (`/` -> `/index.html`). No history fallback, no prefix rules
    # (D-A1-4). "/api/comments" has no trailing slash, so it is never
    # rewritten and always falls through to routes.

    if req.headers[If-None-Match] == quoted(entry.hash):
        return Ok(Some(304 + ETag + Cache-Control, empty body))

    headers = Content-Type   : entry.content_type
              Content-Length : entry.len          # known at unpack, no buffering
              ETag           : quoted(entry.hash)
              Cache-Control  : cache_control_for(&entry.content_type)

    if method == HEAD: return Ok(Some(200 + headers, empty body))

    # F3: never instantiates the component
    download_id = dispatch_native(.., "blob-store", "open-download",
                                  {"hash": entry.hash, "offset": 0})
    body = StreamBody::new(stream::unfold(BlobDownloadState{..}, blob_download_step))
    Ok(Some(200 + headers + body))
```

**Cache-Control is chosen by content type, not by path.** Revision 1 compared
the request path to the SPA fallback, which the review correctly showed was
inverted. With the fallback gone the rule is simply: `text/html` gets
`no-cache` (its name is stable while its content changes every deploy, so
caching it immutably pins users to an old bundle); everything else gets
`public, max-age=31536000, immutable`, which is correct for bundler-hashed
filenames.

---

## §6 Phases

| Phase | Scope | Done when |
|---|---|---|
| **P1** | Types + deps + R3-A's `match_path` move. **Confirm** F2's 3.57× arithmetic against a real deploy (no longer a gate — the number is settled on paper) | `cargo check --workspace` clean; measured expansion within a few percent of 3.57×, or a finding if not |
| **P2** | `assets.rs`: `unpack_asset_bundle` (accumulating into `written`), `store_manifest`, `hashes_of`, `delete_hashes`. **No rollback here** — that is the caller's scope guard in P3. Unit-tested against `ObjectStoreBlobProvider::in_memory` | Every cap, traversal, collision, and quota case tested; `written` correct on the error path; `delete_hashes` tested as a pure set difference; no deploy wiring |
| **P3** | Deploy/undeploy/redeploy wiring: call sites 4-11 | Manifest in the registry; undeploy and redeploy both delete the prior generation's blobs |
| **P4** | Serving: `try_handle_asset`, call sites 12, 15 | Integration test fetches an asset over the HTTP bridge |
| **P5** | Failure matrix + exit criteria tests | §7 complete |

P1's measurement no longer gates `D-A1-5` — revision 3 settled the number on
paper. Still run it: a measured expansion materially off 3.57× means an encoding
assumption is wrong somewhere, which is worth knowing in P1 rather than P4.

---

## §7 Tests

Covering **A1's rows only**. Matrix rows 5, 6 and 8, and exit criteria 2, 5, 6,
7, belong to A2/A3/A4 — revision 1's "every row is covered" was wrong.

| Test | Covers |
|---|---|
| `../escape.txt`, non-UTF-8 name, `/blobs/x`, and a path colliding with a declared route each rejected at deploy, no blob written | Matrix 1, F7, D-A1-4 |
| over compressed cap / over unpacked cap (aborts mid-read) / over file count — each leaves no blob behind | Matrix 2, D-A1-5 |
| blob quota failure mid-unpack rolls back every earlier blob | D-A1-10 |
| unknown path → 404, **and** no `blob-store` dispatch occurred, **and** instantiation delta is 0 | Matrix 3 (all three clauses) |
| service B cannot fetch service A's asset | Matrix 4 |
| `visibility: private` → 404 byte-identical to no bundle | Matrix 7, D-A1-8 |
| `GET /index.html` 200, correct content type; instantiation **delta** across the request is 0 | Exit 3, D-A1-7 |
| second GET with `If-None-Match` → 304, empty body | Exit 4 |
| `HEAD` returns headers, no body, same ETag | Smaller finding |
| multi-chunk asset round-trips byte-identical | §5.1/§5.2 |
| `text/html` gets `no-cache`; hashed `.js` gets `immutable` | §5.2 |
| **`GET /` returns index.html**, and `GET /sub/` returns `/sub/index.html` | D-A1-11, task.md reference scenario step 4 |
| `GET /nofallback` (no trailing slash, not an entry) → 404, **not** index.html | D-A1-11 boundary |
| **redeploy where most files are unchanged: every shared hash survives**, only genuinely removed files are deleted, and the site still serves afterwards | D-A1-9 forward, revision-3 finding 2 |
| **deploy failing after unpack** (simulate the `:1749` path): newly written blobs are gone, **and every blob the still-live old manifest references survives** | D-A1-9 backward, finding 5, R3-B |
| an asset at `/docs/intro.html` is refused when `GET /docs/{slug}` is declared | D-A1-4, revision-3 finding 3 |
| the same asset is **accepted** when only `POST /docs/{slug}` is declared | D-A1-4 method filter, revision-4 finding 4 |
| `unpack_asset_bundle` failing midway leaves `written` populated with exactly the hashes it stored — tested standalone, without a deploy | §3.7 out-param split, revision-4 finding 3 |
| `delete_hashes(remove, keep)` deletes exactly `remove − keep`, tested as a pure set operation in both directions | D-A1-9, R3-B |
| a bundle that fits `MAX_ASSET_BUNDLE_BYTES` but not the frame beside its component is refused **client-side**, naming both encoded sizes | D-A1-5, R3-C |
| deploy logs entry count and resolved visibility | D-A1-1, revision-3 finding 6 |
| undeploy removes registry entry and every blob | Call site 7 |
| `POST /api/x` and `GET /api/x` reach routes, not assets, when a bundle is present | Revision-2 finding 1 |

Exit criterion 1 ("deploys as a single WASM component") is **not** an A1 test —
it is a property of A3's fixture. Revision 1 mapped it to a content-type test
that did not test it.

---

## §8 What this plan does not decide

1. **Bypassing native dispatch for asset reads** (F4, `D-A1-6`). Backlog row
   owed when this slice lands, whether or not a benchmark is run.
2. **Boot-time manifest loading** (`D-A1-2`, F10). Backlog row owed: *asset
   manifests do not survive a substrate restart; the blobs do, so a restart
   before undeploy orphans them permanently, and no GC reclaims them.* This is
   the one consequence not shared with `http_routes`, which leaks nothing.
3. **Reviving `artifact-source::url`** (finding 3). Making the substrate fetch
   an archive from the network at deploy raises SSRF and egress-policy
   questions that are their own decision. Backlog row owed; it is also the
   natural escape hatch from `D-A1-5`'s cap.
4. **Chunked artifact upload.** The real fix for F2. Out of A1; likely its own
   slice, and the thing to build if a real bundle exceeds `D-A1-5`'s combined
   frame budget — which, at 3.57× and beside a 2 MiB component, it will do at
   roughly 2.4 MiB compressed.
5. **Range requests, and wire compression.** `open_download` already takes an
   offset so `Range:` is nearly free, and serving pre-compressed blobs with
   `Content-Encoding: gzip` is a real win for text — neither is required by
   task.md and neither would be tested.
6. **How A1's `asset-bundle.visibility` reconciles with ADR-0018's
   `service-config.visibility`** when that ADR is accepted. They answer
   different questions and both may be wanted; the shared enum is the hedge.
7. **Content addressing spans the asset/non-asset boundary.** `put_blob` keys
   one object per `(service_id, hash)` across *all* of a service's blobs, with
   no namespace separating deploy-written assets from guest-written content and
   no refcount. So if a guest ever stores content byte-identical to an asset
   that a later redeploy removes, `D-A1-9`'s forward delete takes the guest's
   copy too. Inherent to the existing store rather than introduced by A1, and
   vanishingly unlikely in practice — recorded as a known limitation rather
   than engineered around, because the fixes (an asset key namespace, or
   refcounting) are both larger than the risk.

---

## §9 Corrections owed to task.md

1. *Migration impact* bullet 3 — A1 does add WIT (`visibility`, `asset-bundle`,
   `service-config.assets`).
2. Failure-matrix row 2 — should name `D-A1-5`'s three caps.
3. The third open design point (DEK-scoped bundle) is resolved by `D-A1-3`.
4. The first open design point ("deploy writes the manifest as its own blob and
   puts that hash in the config, with the router loading and caching it per
   service") — the **loading** half is explicitly not built; `D-A1-2` and §8.2
   record why.
5. A1's scope line should say **exact-path static serving plus a trailing-slash
   directory index**, so SPA history fallback is visibly A3's problem while
   `GET /` visibly is not.
6. Failure-matrix row 2's cap wording should point at `D-A1-5`'s *combined*
   client-side check, not only at a fixed byte cap (R3-C).
