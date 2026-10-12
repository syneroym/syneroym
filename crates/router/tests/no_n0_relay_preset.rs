//! Guards against using public n0 relay presets in runtime code.
//!
//! The endpoint must never fall back to the public n0 relays, which show
//! metadata to a third party. All runtime endpoints must use custom relay maps
//! or start with an empty relay map.

use std::{
    fs,
    path::{Path, PathBuf},
};

const FORBIDDEN_PATTERNS: &[&str] = &[
    "presets",
    "RelayMode::Default",
    "RelayMode::Staging",
    "Endpoint::builder(",
    "Endpoint::bind(",
];

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn scan_source_root(base: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == "smoke-tests") {
            continue;
        }
        let src_dir = path.join("src");
        if src_dir.is_dir() {
            collect_rs_files(&src_dir, files);
        }
    }
}

#[test]
fn no_n0_relay_presets_in_runtime_source() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    scan_source_root(&workspace_root.join("crates"), &mut files);
    scan_source_root(&workspace_root.join("apps"), &mut files);

    let mut violations = Vec::new();
    for file in files {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        for (line_idx, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for pattern in FORBIDDEN_PATTERNS {
                if line.contains(pattern) {
                    let relative_path =
                        file.strip_prefix(&workspace_root).unwrap_or(&file).display();
                    violations.push(format!(
                        "{relative_path}:{}: found pattern {pattern:?}",
                        line_idx + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found forbidden n0 relay patterns in runtime source:\n{}",
        violations.join("\n")
    );
}
