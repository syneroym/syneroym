//! Deploy-time document primitives shared by the control plane and the
//! container engine.
//!
//! A deploy manifest names text documents (a JSON Schema, an FDAE policy, a
//! container's config files) that either travel inside the deploy call or live
//! on the substrate host's own filesystem. The host-side arm is an arbitrary
//! file read driven by a remote caller, so its guards live here rather than at
//! each call site: the control plane and the Podman engine both need them, and
//! a guard that exists in two copies eventually exists in one.
//!
//! This module deliberately knows nothing about the WIT types. Callers match
//! the two-arm source variant themselves and call in for the host-side arm,
//! which keeps `syneroym-core` free of a `syneroym-wit-interfaces` dependency.

use std::{
    fs,
    path::{Component, Path},
};

/// Upper bound on a single deploy-time document, applied to both arms.
///
/// Inline content is bounded because a deploy manifest is not a blob store --
/// `artifact-source::url` exists for anything large. Host-side reads are
/// bounded by the same number for a different reason: the path is chosen by a
/// remote caller, so an unbounded `read_to_string` is a memory-exhaustion
/// lever. 1 MiB is far above any real JSON Schema, ReBAC policy, or container
/// config file.
pub const MAX_DEPLOY_DOCUMENT_BYTES: u64 = 1024 * 1024;

/// Upper bound on the combined size of one container volume's files.
///
/// [`MAX_DEPLOY_DOCUMENT_BYTES`] alone bounds each file but not how many, and
/// a volume is the one place a manifest can name an unbounded number of them.
pub const MAX_VOLUME_TOTAL_BYTES: u64 = 4 * 1024 * 1024;

/// Rejects `path` if it's absolute, contains a `..` component, or -- once
/// symlinks are resolved -- canonicalizes to somewhere outside the process's
/// working directory. The component check alone doesn't catch a symlink placed
/// under the working directory that itself points outside it (no `..` anywhere
/// in `path`), so the second, filesystem-resolving check is not redundant.
/// `field_name` names the offending manifest field for the error message.
pub fn reject_path_escape(path: &Path, field_name: &str) -> Result<(), String> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) || path.is_absolute() {
        return Err(format!(
            "Arbitrary file read prevented: Path traversal or absolute paths are not allowed in \
             {field_name}: {:?}",
            path
        ));
    }

    let cwd = std::env::current_dir()
        .map_err(|e| format!("Failed to resolve working directory: {}", e))?;
    let canonical_cwd = fs::canonicalize(&cwd)
        .map_err(|e| format!("Failed to resolve working directory: {}", e))?;
    let resolved = fs::canonicalize(cwd.join(path))
        .map_err(|e| format!("Failed to resolve {field_name} at {}: {}", path.display(), e))?;
    if !resolved.starts_with(&canonical_cwd) {
        return Err(format!(
            "Arbitrary file read prevented: {field_name} resolves outside the working directory \
             via a symlink: {:?}",
            path
        ));
    }
    Ok(())
}

/// Reads a deploy-time document from the substrate host's filesystem, guarded
/// against traversal and capped at [`MAX_DEPLOY_DOCUMENT_BYTES`].
///
/// The size is checked against the file's metadata before the read, so an
/// oversized file is rejected without ever being loaded.
pub fn read_host_document(path: &Path, field_name: &str) -> Result<String, String> {
    reject_path_escape(path, field_name)?;

    let metadata = fs::metadata(path)
        .map_err(|e| format!("Failed to read {field_name} at {}: {}", path.display(), e))?;
    if metadata.len() > MAX_DEPLOY_DOCUMENT_BYTES {
        return Err(format!(
            "{field_name} at {} is {} bytes, exceeding the {} byte limit",
            path.display(),
            metadata.len(),
            MAX_DEPLOY_DOCUMENT_BYTES
        ));
    }

    fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {field_name} at {}: {}", path.display(), e))
}

/// Enforces [`MAX_DEPLOY_DOCUMENT_BYTES`] on caller-supplied inline content.
pub fn check_inline_size(content: &str, field_name: &str) -> Result<(), String> {
    if content.len() as u64 > MAX_DEPLOY_DOCUMENT_BYTES {
        return Err(format!(
            "inline {field_name} is {} bytes, exceeding the {} byte limit",
            content.len(),
            MAX_DEPLOY_DOCUMENT_BYTES
        ));
    }
    Ok(())
}

/// Rejects a path that is meant to stay inside a directory the substrate owns
/// (a container volume root).
///
/// Purely lexical, and deliberately so: it runs *before* anything is written,
/// on a name supplied by a remote caller, and it must not depend on the target
/// existing. Callers still resolve the result against the volume root, which
/// catches what a lexical check cannot -- the two guards fail differently and
/// both are wanted.
pub fn reject_relative_escape(relative_path: &str, field_name: &str) -> Result<(), String> {
    let path = Path::new(relative_path);
    let mut has_content = false;

    for component in path.components() {
        match component {
            Component::Normal(_) => has_content = true,
            // `.` is harmless and normalizes away.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{field_name} must be a relative path inside the volume, with no '..', root, \
                     or drive prefix: {:?}",
                    relative_path
                ));
            }
        }
    }

    if !has_content {
        return Err(format!("{field_name} must not be empty: {:?}", relative_path));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn reject_path_escape_rejects_parent_dir() {
        let err = reject_path_escape(Path::new("../secrets.json"), "schema").unwrap_err();
        assert!(err.contains("Path traversal or absolute paths are not allowed"), "{err}");
    }

    #[test]
    fn reject_path_escape_rejects_nested_parent_dir() {
        let err = reject_path_escape(Path::new("cfg/../../secrets.json"), "schema").unwrap_err();
        assert!(err.contains("Path traversal or absolute paths are not allowed"), "{err}");
    }

    #[test]
    fn reject_path_escape_rejects_absolute() {
        let err = reject_path_escape(Path::new("/etc/passwd"), "schema").unwrap_err();
        assert!(err.contains("Path traversal or absolute paths are not allowed"), "{err}");
    }

    /// The lexical `..` check passes here -- the escape is entirely inside the
    /// symlink target, which is why the canonicalizing second check exists.
    #[test]
    fn reject_path_escape_rejects_symlink_escaping_cwd() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secrets.json");
        fs::write(&target, "{}").unwrap();

        let link_name = format!("test_deploy_docs_symlink_{}.json", std::process::id());
        std::os::unix::fs::symlink(&target, &link_name).unwrap();

        let result = reject_path_escape(Path::new(&link_name), "schema");
        let _ = fs::remove_file(&link_name);

        let err = result.unwrap_err();
        assert!(err.contains("resolves outside the working directory via a symlink"), "{err}");
    }

    /// Written inside the working directory, because an absolute path would be
    /// turned away by the traversal guard long before the size check -- which
    /// would make this test pass for the wrong reason.
    #[test]
    fn read_host_document_rejects_oversize() {
        let name = format!("test_deploy_docs_big_{}.json", std::process::id());
        let mut f = fs::File::create(&name).unwrap();
        f.write_all(&vec![b'x'; MAX_DEPLOY_DOCUMENT_BYTES as usize + 1]).unwrap();
        drop(f);

        let result = read_host_document(Path::new(&name), "schema");
        let _ = fs::remove_file(&name);

        let err = result.unwrap_err();
        assert!(err.contains("exceeding the"), "{err}");
    }

    #[test]
    fn read_host_document_reads_a_document_under_the_cap() {
        let name = format!("test_deploy_docs_ok_{}.json", std::process::id());
        fs::write(&name, r#"{"type":"object"}"#).unwrap();

        let result = read_host_document(Path::new(&name), "schema");
        let _ = fs::remove_file(&name);

        assert_eq!(result.unwrap(), r#"{"type":"object"}"#);
    }

    #[test]
    fn check_inline_size_accepts_at_limit_and_rejects_above() {
        let at_limit = "x".repeat(MAX_DEPLOY_DOCUMENT_BYTES as usize);
        assert!(check_inline_size(&at_limit, "schema").is_ok());

        let over = "x".repeat(MAX_DEPLOY_DOCUMENT_BYTES as usize + 1);
        let err = check_inline_size(&over, "schema").unwrap_err();
        assert!(err.contains("exceeding the"), "{err}");
    }

    #[test]
    fn reject_relative_escape_accepts_nested_relative_paths() {
        assert!(reject_relative_escape("default.conf", "volume file").is_ok());
        assert!(reject_relative_escape("certs/ca.pem", "volume file").is_ok());
        assert!(reject_relative_escape("./default.conf", "volume file").is_ok());
    }

    #[test]
    fn reject_relative_escape_rejects_traversal_and_absolute() {
        for bad in ["../escape.conf", "certs/../../escape.conf", "/etc/passwd"] {
            let err = reject_relative_escape(bad, "volume file").unwrap_err();
            assert!(err.contains("must be a relative path inside the volume"), "{bad}: {err}");
        }
    }

    #[test]
    fn reject_relative_escape_rejects_empty() {
        for bad in ["", ".", "./"] {
            let err = reject_relative_escape(bad, "volume file").unwrap_err();
            assert!(err.contains("must not be empty"), "{bad}: {err}");
        }
    }
}
