//! Unpacks a deployed service's static asset bundle into per-file blobs
//! (M06A A1) and the deletion helper that keeps a redeploy/undeploy from
//! either leaking the previous generation's blobs or destroying the still-
//! live one (`D-A1-9`).
//!
//! `unpack_asset_bundle` is deliberately the only place that reads the
//! archive; everything downstream (the router's serving path, the deploy
//! wiring) only ever sees the resulting [`AssetManifest`].

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path},
    sync::Arc,
};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use syneroym_core::{
    asset_manifest::{AssetEntry, AssetManifest},
    deploy_docs::{
        MAX_ASSET_BUNDLE_BYTES, MAX_ASSET_FILE_COUNT, MAX_ASSET_UNPACKED_BYTES,
        reject_archive_entry_path,
    },
    http_routes::{HttpRoute, match_path},
};
use syneroym_data_blob::BlobProvider;
use zeroize::Zeroizing;

/// Read size for the incremental unpacked-size check: the cap must be
/// enforced *during* decompression, not after a whole entry has already
/// been expanded into memory (a compressed-only cap is a decompression-bomb
/// lever).
const UNPACK_READ_CHUNK_BYTES: usize = 64 * 1024;

/// The `/blobs/{hash}` prefix is a fixed, always-intercepted route
/// (`crates/router/src/route_handler/http.rs`); an asset bundle claiming a
/// path under it would be permanently unreachable (shadowed) rather than
/// served, so it is refused at deploy instead.
const RESERVED_BLOBS_PREFIX: &str = "/blobs/";

/// Unpacks `archive` (a gzip-compressed tar of a site root) into per-file
/// blobs and returns the resulting manifest.
///
/// **Writes every hash it stores into `written` as it goes, including on
/// the error path.** This function performs no deletion of its own: the
/// caller owns `written` and the scope guard around it, which is also the
/// only place that can see the still-live previous manifest and so the
/// only place that can compute what to keep (M06A D-A1-9, R3-B).
///
/// `declared_routes` are the service's own `http_routes` *patterns*, not
/// literal paths, filtered to `GET`/`HEAD` by the caller's own declared
/// method -- checked with `match_path` so a parameterised route (`GET
/// /docs/{slug}`) correctly collides with a literal asset path
/// (`/docs/intro.html`) despite neither string equalling the other
/// (D-A1-4). Every `.../index.html` entry is also checked against its
/// D-A1-11 directory form (`/docs/`, or `/` at the root), since that path
/// is answerable from the bundle too and a route pattern that only matches
/// the directory form would otherwise deploy clean.
pub async fn unpack_asset_bundle(
    service_id: &str,
    archive: &[u8],
    declared_hash: Option<&str>,
    declared_routes: &[HttpRoute],
    blob: &Arc<dyn BlobProvider>,
    dek: Option<Zeroizing<[u8; 32]>>,
    written: &mut BTreeSet<String>,
) -> Result<AssetManifest, String> {
    if archive.len() as u64 > MAX_ASSET_BUNDLE_BYTES {
        return Err(format!(
            "asset bundle archive is {} bytes, exceeding the {MAX_ASSET_BUNDLE_BYTES} byte limit",
            archive.len()
        ));
    }
    if let Some(expected) = declared_hash {
        let actual = hex::encode(Sha256::digest(archive));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "asset bundle archive hash mismatch: declared {expected}, computed {actual}"
            ));
        }
    }

    let get_head_routes: Vec<HttpRoute> = declared_routes
        .iter()
        .filter(|r| r.method.eq_ignore_ascii_case("GET") || r.method.eq_ignore_ascii_case("HEAD"))
        .cloned()
        .collect();

    // `tar::Archive`/`Entries` hold their reader behind a `RefCell`, so
    // neither is `Send` -- this whole phase must stay synchronous, with the
    // archive fully consumed and dropped before the first `.await` below,
    // or every caller up to the JSON-RPC dispatch trait's `async_trait`
    // bound stops compiling (its futures must be `Send`). It is also
    // CPU-bound (gzip inflation of up to `MAX_ASSET_UNPACKED_BYTES`), so it
    // runs on a blocking-pool thread rather than stalling a tokio worker
    // for the whole deploy -- the same treatment schema validation already
    // gets in `orchestration.rs`. `archive` is bounded at
    // `MAX_ASSET_BUNDLE_BYTES` (2 MiB), so the clone into the blocking
    // task's owned, `'static` closure is cheap.
    let archive_owned = archive.to_vec();
    let collected = tokio::task::spawn_blocking(move || {
        let route_refs: Vec<&HttpRoute> = get_head_routes.iter().collect();
        parse_and_validate_entries(&archive_owned, &route_refs)
    })
    .await
    .map_err(|e| format!("asset bundle parsing task panicked: {e}"))??;

    let mut entries = BTreeMap::new();
    for (key, content, content_type) in collected {
        let len = content.len() as u64;
        let hash = blob
            .put_blob(service_id, content, dek.clone())
            .await
            .map_err(|e| format!("failed to store asset {key:?}: {e}"))?;
        // Recorded before any further fallible step in this iteration --
        // the caller's rollback must see every blob this call actually
        // wrote, even one whose entry is never inserted below.
        written.insert(hash.clone());
        entries.insert(key, AssetEntry { hash, len, content_type });
    }

    Ok(AssetManifest { entries })
}

/// The synchronous half of [`unpack_asset_bundle`]: decompresses, validates,
/// and fully reads every accepted entry into memory, in path order. No I/O
/// beyond the in-memory `archive` slice -- kept separate from the async
/// blob-writing loop so the non-`Send` `tar::Archive` never lives across an
/// `.await` point.
fn parse_and_validate_entries(
    archive: &[u8],
    get_head_routes: &[&HttpRoute],
) -> Result<Vec<(String, Vec<u8>, String)>, String> {
    let mut collected = Vec::new();
    let mut total: u64 = 0;
    let mut seen_keys = BTreeSet::new();

    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let tar_entries =
        tar.entries().map_err(|e| format!("failed to read asset bundle archive: {e}"))?;

    for entry in tar_entries {
        let mut entry = entry.map_err(|e| format!("failed to read asset bundle entry: {e}"))?;
        // Directories and symlinks are skipped (not rejected -- an ordinary
        // `tar czf` of a directory is accepted, and a symlink is never
        // followed, so no canonicalisation against a real filesystem is
        // needed), but "skipped" must still mean "read through the same
        // MAX_ASSET_UNPACKED_BYTES-bounded loop as a real file", just
        // without keeping the bytes: a non-file header can declare an
        // arbitrary data-section size, and `tar`'s own iterator would
        // decompress-and-discard that many bytes when advancing to the
        // next entry regardless of whether this function ever calls
        // `read`. Accounting nothing for a `continue`d entry left that
        // decompression work -- and, at the file-count cap, the entry
        // itself -- off the books.
        let is_file = entry.header().entry_type().is_file();

        let accepted_key = if is_file {
            let raw_path = entry
                .path()
                .map_err(|e| format!("failed to read asset bundle entry path: {e}"))?
                .into_owned();
            let accepted = reject_archive_entry_path(&raw_path, "assets archive entry")?;
            let key = normalize_asset_path(&accepted);

            // Two entries normalising to the same key would otherwise both
            // `put_blob`, with the second silently winning the manifest
            // slot -- the first entry's hash stays in `written` (correctly
            // surviving rollback) but no manifest entry ever points to it,
            // orphaning that blob forever (no boot-time loader, no GC).
            // Rejected at deploy time instead, before either is written.
            if !seen_keys.insert(key.clone()) {
                return Err(format!("asset bundle contains a duplicate path {key:?}"));
            }

            if key.starts_with(RESERVED_BLOBS_PREFIX) {
                return Err(format!(
                    "asset path {key:?} collides with the reserved {RESERVED_BLOBS_PREFIX} prefix"
                ));
            }
            // D-A1-11's directory-index rewrite makes an `.../index.html`
            // entry also answerable at the directory path itself
            // (`/docs/index.html` for `GET /docs/`, `/index.html` for
            // `GET /`) -- a route pattern that only collides with that
            // directory form, not the literal manifest key, would
            // otherwise deploy clean and then split one logical resource
            // across two handlers depending on a trailing slash.
            // `strip_suffix("/index.html")` (rather than "index.html")
            // deliberately requires the path separator, so a file that
            // merely ends in those letters (`/myindex.html`) is not
            // misread as a directory index.
            let directory_form = key.strip_suffix("/index.html").map(|prefix| format!("{prefix}/"));
            for route in get_head_routes {
                if match_path(&route.path, &key).is_some() {
                    return Err(format!(
                        "asset path {key:?} collides with declared route {} {}",
                        route.method, route.path
                    ));
                }
                if let Some(dir) = &directory_form
                    && match_path(&route.path, dir).is_some()
                {
                    return Err(format!(
                        "asset path {key:?} collides with declared route {} {} as directory index \
                         {dir:?}",
                        route.method, route.path
                    ));
                }
            }
            if collected.len() + 1 > MAX_ASSET_FILE_COUNT {
                return Err(format!(
                    "asset bundle contains more than {MAX_ASSET_FILE_COUNT} files"
                ));
            }
            Some(key)
        } else {
            None
        };

        let mut content = Vec::new();
        let mut buf = [0u8; UNPACK_READ_CHUNK_BYTES];
        loop {
            let n = entry.read(&mut buf).map_err(|e| {
                format!(
                    "failed to read asset bundle entry {:?}: {e}",
                    accepted_key.as_deref().unwrap_or("<non-file entry>")
                )
            })?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_ASSET_UNPACKED_BYTES {
                return Err(format!(
                    "asset bundle unpacks to more than {MAX_ASSET_UNPACKED_BYTES} bytes"
                ));
            }
            if is_file {
                content.extend_from_slice(&buf[..n]);
            }
        }

        if let Some(key) = accepted_key {
            let content_type = mime_guess::from_path(&key).first_or_octet_stream().to_string();
            collected.push((key, content, content_type));
        }
    }

    Ok(collected)
}

/// Normalises an archive entry path already accepted by
/// [`reject_archive_entry_path`] into the manifest's own key shape:
/// `.`/`./` segments dropped, components rejoined with `/`, one leading
/// slash. `reject_archive_entry_path` validates but does not reshape --
/// this is the one place that does.
fn normalize_asset_path(accepted: &str) -> String {
    let mut out = String::from("/");
    let mut first = true;
    for component in Path::new(accepted).components() {
        if let Component::Normal(seg) = component {
            if !first {
                out.push('/');
            }
            out.push_str(&seg.to_string_lossy());
            first = false;
        }
    }
    out
}

/// Serializes `manifest` and stores it as its own blob, returning its hash.
pub async fn store_manifest(
    service_id: &str,
    manifest: &AssetManifest,
    blob: &Arc<dyn BlobProvider>,
    dek: Option<Zeroizing<[u8; 32]>>,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|e| format!("failed to serialize asset manifest: {e}"))?;
    blob.put_blob(service_id, bytes, dek)
        .await
        .map_err(|e| format!("failed to store asset manifest: {e}"))
}

/// Every hash `manifest` references, including the manifest blob's own hash
/// when given -- the `remove` or `keep` argument to [`delete_hashes`].
#[must_use]
pub fn hashes_of(manifest: &AssetManifest, manifest_hash: Option<&str>) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = manifest.entries.values().map(|e| e.hash.clone()).collect();
    if let Some(hash) = manifest_hash {
        set.insert(hash.to_string());
    }
    set
}

/// The one deletion helper, used in both directions (D-A1-9). Deletes
/// `remove - keep`, never a hash `keep` contains. **Forward** (successful
/// redeploy): remove the old manifest's hashes, keep the new manifest's.
/// **Backward** (any failure between unpack and registry insert): remove
/// what this deploy wrote, keep the still-live old manifest's.
///
/// Blobs are content-addressed with one object per `(service_id, hash)` and
/// no refcount, so an unchanged file has the *same* hash in both
/// generations -- deleting either manifest's hashes wholesale would destroy
/// live data; subtracting `keep` is what prevents that.
///
/// Attempts every deletion even if one fails, so a single missing/already-
/// gone blob doesn't abandon cleanup of the rest; returns the first error
/// encountered, if any, after every attempt has run.
pub async fn delete_hashes(
    service_id: &str,
    remove: &BTreeSet<String>,
    keep: &BTreeSet<String>,
    blob: &Arc<dyn BlobProvider>,
) -> Result<(), String> {
    let mut first_err = None;
    for hash in remove.difference(keep) {
        if let Err(e) = blob.delete_blob(service_id, hash).await {
            let msg = format!("failed to delete asset blob {hash}: {e}");
            if first_err.is_none() {
                first_err = Some(msg);
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::GzEncoder};
    use syneroym_data_blob::object_store_impl::ObjectStoreBlobProvider;

    use super::*;

    const NO_ROUTES: &[HttpRoute] = &[];

    /// Writes an entry's raw header bytes directly, bypassing `tar::Header::
    /// set_path`'s own traversal/absolute-path safety check -- several tests
    /// below deliberately construct an archive an *attacker* would produce,
    /// which that check would otherwise refuse to let this test helper build
    /// in the first place.
    fn write_entry(builder: &mut tar::Builder<Vec<u8>>, path: &str, data: &[u8]) {
        let mut header = tar::Header::new_gnu();
        let name_field = &mut header.as_old_mut().name;
        let name_bytes = path.as_bytes();
        let n = name_bytes.len().min(name_field.len());
        name_field[..n].copy_from_slice(&name_bytes[..n]);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }

    fn make_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in files {
            write_entry(&mut builder, path, data);
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn provider() -> Arc<dyn BlobProvider> {
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None))
    }

    #[tokio::test]
    async fn duplicate_normalized_paths_are_rejected_with_nothing_written() {
        // `./index.html` and `index.html` normalise to the same manifest
        // key -- without the check, the second `put_blob` would silently
        // win the manifest slot and orphan the first blob forever.
        let archive = make_archive(&[("./index.html", b"one"), ("index.html", b"two")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("duplicate path"), "{err}");
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn unpacks_a_small_bundle_into_blobs() {
        let archive = make_archive(&[
            ("index.html", b"<html>hi</html>"),
            ("assets/app.js", b"console.log(1)"),
        ]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let manifest =
            unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
                .await
                .unwrap();

        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(written.len(), 2);
        let index = manifest.entries.get("/index.html").unwrap();
        assert_eq!(index.len, "<html>hi</html>".len() as u64);
        assert_eq!(index.content_type, "text/html");
        let js = manifest.entries.get("/assets/app.js").unwrap();
        assert!(js.content_type.contains("javascript"));

        let stored = blob.get_blob("svc", &index.hash, None).await.unwrap();
        assert_eq!(stored, b"<html>hi</html>");
    }

    #[tokio::test]
    async fn declared_hash_mismatch_is_rejected_before_any_blob_is_written() {
        let archive = make_archive(&[("index.html", b"hi")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle(
            "svc",
            &archive,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            NO_ROUTES,
            &blob,
            None,
            &mut written,
        )
        .await
        .unwrap_err();
        assert!(err.contains("hash mismatch"), "{err}");
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn declared_hash_match_is_accepted() {
        let archive = make_archive(&[("index.html", b"hi")]);
        let expected = hex::encode(Sha256::digest(&archive));
        let blob = provider();
        let mut written = BTreeSet::new();
        unpack_asset_bundle("svc", &archive, Some(&expected), NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap();
        assert_eq!(written.len(), 1);
    }

    #[tokio::test]
    async fn traversal_entry_is_rejected_with_nothing_written() {
        let archive = make_archive(&[("../escape.txt", b"evil")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("relative path"), "{err}");
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn absolute_path_entry_is_rejected() {
        let archive = make_archive(&[("/etc/passwd", b"evil")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("relative path"), "{err}");
    }

    #[tokio::test]
    async fn blobs_prefix_is_rejected() {
        let archive = make_archive(&[("blobs/x", b"evil")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }

    #[tokio::test]
    async fn path_colliding_with_a_declared_get_route_is_rejected() {
        let archive = make_archive(&[("docs/intro.html", b"hi")]);
        let routes = [HttpRoute {
            method: "GET".to_string(),
            path: "/docs/{slug}".to_string(),
            target: "data-layer".to_string(),
            operation: "get".to_string(),
            collection: Some("docs".to_string()),
            topic: None,
            protocol: None,
            public: false,
        }];
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, &routes, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("collides with declared route"), "{err}");
    }

    #[tokio::test]
    async fn directory_index_form_colliding_with_a_declared_get_route_is_rejected() {
        // `docs/index.html` doesn't literally equal `/docs`, but D-A1-11
        // makes it answerable at `GET /docs/` too -- the collision check
        // must catch that even though `match_path` alone (segment-count
        // equality) would not.
        let archive = make_archive(&[("docs/index.html", b"hi")]);
        let routes = [HttpRoute {
            method: "GET".to_string(),
            path: "/docs".to_string(),
            target: "data-layer".to_string(),
            operation: "get".to_string(),
            collection: Some("docs".to_string()),
            topic: None,
            protocol: None,
            public: false,
        }];
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, &routes, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("collides with declared route"), "{err}");
        assert!(err.contains("directory index"), "{err}");
    }

    #[tokio::test]
    async fn root_directory_index_colliding_with_a_declared_get_route_is_rejected() {
        let archive = make_archive(&[("index.html", b"hi")]);
        let routes = [HttpRoute {
            method: "GET".to_string(),
            path: "/".to_string(),
            target: "data-layer".to_string(),
            operation: "get".to_string(),
            collection: Some("root".to_string()),
            topic: None,
            protocol: None,
            public: false,
        }];
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, &routes, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("collides with declared route"), "{err}");
    }

    #[tokio::test]
    async fn a_file_merely_ending_in_index_html_is_not_treated_as_a_directory_index() {
        // `myindex.html` ends in the letters "index.html" but has no path
        // separator before them, so it must not be misread as
        // `/my/index.html`'s directory form.
        let archive = make_archive(&[("myindex.html", b"hi")]);
        let routes = [HttpRoute {
            method: "GET".to_string(),
            path: "/my".to_string(),
            target: "data-layer".to_string(),
            operation: "get".to_string(),
            collection: Some("my".to_string()),
            topic: None,
            protocol: None,
            public: false,
        }];
        let blob = provider();
        let mut written = BTreeSet::new();
        unpack_asset_bundle("svc", &archive, None, &routes, &blob, None, &mut written)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn path_matching_only_a_post_route_is_accepted() {
        let archive = make_archive(&[("docs/intro.html", b"hi")]);
        let routes = [HttpRoute {
            method: "POST".to_string(),
            path: "/docs/{slug}".to_string(),
            target: "data-layer".to_string(),
            operation: "put".to_string(),
            collection: Some("docs".to_string()),
            topic: None,
            protocol: None,
            public: false,
        }];
        let blob = provider();
        let mut written = BTreeSet::new();
        unpack_asset_bundle("svc", &archive, None, &routes, &blob, None, &mut written)
            .await
            .unwrap();
        assert_eq!(written.len(), 1);
    }

    #[tokio::test]
    async fn over_file_count_cap_is_rejected() {
        let files: Vec<(String, Vec<u8>)> =
            (0..MAX_ASSET_FILE_COUNT + 1).map(|i| (format!("f{i}.txt"), b"x".to_vec())).collect();
        let refs: Vec<(&str, &[u8])> =
            files.iter().map(|(p, d)| (p.as_str(), d.as_slice())).collect();
        let archive = make_archive(&refs);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("more than"), "{err}");
    }

    #[tokio::test]
    async fn over_unpacked_cap_aborts_mid_read_and_stops_writing() {
        let big = vec![b'x'; MAX_ASSET_UNPACKED_BYTES as usize + 1];
        let archive = make_archive(&[("big.bin", &big)]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("unpacks to more than"), "{err}");
        // The oversized entry is never finished reading, so `put_blob` is
        // never reached for it -- nothing was written.
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn oversized_non_file_entry_is_still_bounded_by_the_unpacked_cap() {
        // A directory entry with a manipulated, oversized declared data
        // section -- the vector the `continue`-before-counting bug left
        // open: `tar`'s own iterator decompresses that many bytes to skip
        // past the entry regardless of whether this function looks at it,
        // so it must count against MAX_ASSET_UNPACKED_BYTES the same as a
        // real file's content would. All-zero data compresses well enough
        // that the archive itself still fits under MAX_ASSET_BUNDLE_BYTES.
        let big_size = MAX_ASSET_UNPACKED_BYTES as usize + 1;
        let mut builder = tar::Builder::new(Vec::new());
        let mut dir_header = tar::Header::new_gnu();
        let name_field = &mut dir_header.as_old_mut().name;
        name_field[.."evil/".len()].copy_from_slice(b"evil/");
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(big_size as u64);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder.append(&dir_header, vec![0u8; big_size].as_slice()).unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        let archive = encoder.finish().unwrap();
        assert!(
            archive.len() as u64 <= MAX_ASSET_BUNDLE_BYTES,
            "test archive must pass the compressed-size cap to reach the code path under test"
        );

        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("unpacks to more than"), "{err}");
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn over_compressed_bundle_cap_is_rejected_before_any_read() {
        let archive = vec![0u8; MAX_ASSET_BUNDLE_BYTES as usize + 1];
        let blob = provider();
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("exceeding the"), "{err}");
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn blob_quota_failure_mid_unpack_leaves_written_populated_with_earlier_hashes() {
        // A one-blob quota: the first file fits, the second doesn't.
        let blob: Arc<dyn BlobProvider> = Arc::new(ObjectStoreBlobProvider::in_memory(16, None));
        let archive = make_archive(&[("a.txt", b"short"), ("b.txt", b"this one is too long")]);
        let mut written = BTreeSet::new();
        let err = unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
            .await
            .unwrap_err();
        assert!(err.contains("failed to store asset"), "{err}");
        // `a.txt` was written before `b.txt` failed -- the caller's rollback
        // (not this function) is responsible for deleting it.
        assert_eq!(written.len(), 1);
    }

    #[tokio::test]
    async fn directories_and_symlinks_are_skipped_not_rejected() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut dir_header = tar::Header::new_gnu();
        let name_field = &mut dir_header.as_old_mut().name;
        name_field[.."assets/".len()].copy_from_slice(b"assets/");
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder.append(&dir_header, &b""[..]).unwrap();

        write_entry(&mut builder, "assets/a.txt", b"hi");

        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        let archive = encoder.finish().unwrap();

        let blob = provider();
        let mut written = BTreeSet::new();
        let manifest =
            unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
                .await
                .unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries.contains_key("/assets/a.txt"));
    }

    #[tokio::test]
    async fn store_manifest_and_hashes_of_round_trip() {
        let archive = make_archive(&[("index.html", b"hi")]);
        let blob = provider();
        let mut written = BTreeSet::new();
        let manifest =
            unpack_asset_bundle("svc", &archive, None, NO_ROUTES, &blob, None, &mut written)
                .await
                .unwrap();
        let manifest_hash = store_manifest("svc", &manifest, &blob, None).await.unwrap();

        let hashes = hashes_of(&manifest, Some(&manifest_hash));
        assert_eq!(hashes.len(), 2); // one entry + the manifest blob itself
        assert!(hashes.contains(&manifest_hash));
        for entry in manifest.entries.values() {
            assert!(hashes.contains(&entry.hash));
        }
    }

    #[test]
    fn delete_hashes_is_a_pure_set_difference() {
        // Exercised synchronously via `hashes_of`'s output shape; the actual
        // deletion path is covered by the two async tests below, which need
        // a real `BlobProvider` to assert against.
        let remove: BTreeSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let keep: BTreeSet<String> = ["b"].iter().map(|s| s.to_string()).collect();
        let expected: BTreeSet<String> = ["a", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(remove.difference(&keep).cloned().collect::<BTreeSet<_>>(), expected);
    }

    #[tokio::test]
    async fn delete_hashes_forward_keeps_shared_hashes() {
        let blob = provider();
        let shared = blob.put_blob("svc", b"shared".to_vec(), None).await.unwrap();
        let old_only = blob.put_blob("svc", b"old only".to_vec(), None).await.unwrap();

        let remove: BTreeSet<String> = [shared.clone(), old_only.clone()].into_iter().collect();
        let keep: BTreeSet<String> = [shared.clone()].into_iter().collect();
        delete_hashes("svc", &remove, &keep, &blob).await.unwrap();

        assert!(blob.get_blob("svc", &shared, None).await.is_ok(), "shared hash must survive");
        assert!(blob.get_blob("svc", &old_only, None).await.is_err(), "old-only hash must be gone");
    }

    #[tokio::test]
    async fn delete_hashes_backward_keeps_the_still_live_old_manifest() {
        let blob = provider();
        let old_live = blob.put_blob("svc", b"old live".to_vec(), None).await.unwrap();
        let newly_written = blob.put_blob("svc", b"newly written".to_vec(), None).await.unwrap();

        // Rollback: remove what this failed deploy wrote, keep the old
        // manifest's hashes.
        let remove: BTreeSet<String> = [newly_written.clone()].into_iter().collect();
        let keep: BTreeSet<String> = [old_live.clone()].into_iter().collect();
        delete_hashes("svc", &remove, &keep, &blob).await.unwrap();

        assert!(blob.get_blob("svc", &old_live, None).await.is_ok(), "old manifest must survive");
        assert!(
            blob.get_blob("svc", &newly_written, None).await.is_err(),
            "the failed deploy's own write must be gone"
        );
    }
}
