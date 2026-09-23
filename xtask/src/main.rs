use std::{
    cmp::Reverse,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::Value;
use sysinfo::System;
use walkdir::WalkDir;

fn get_git_commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn get_sys_info() -> (String, String, String) {
    let mut sys = System::new_all();
    sys.refresh_all();
    let os = System::long_os_version().unwrap_or_else(|| "Unknown OS".to_string());
    let cpu = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "Unknown CPU".to_string());
    let memory_gb = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let memory = format!("{memory_gb:.1} GB");
    (os, cpu, memory)
}

struct AllowedDependencies<'a> {
    siblings: &'a [&'a str],
    target_independent: &'a [&'a str],
    wasm32: &'a [&'a str],
    native: &'a [&'a str],
}

fn check_crate_manifest_dependencies(
    dir: &Path,
    allowed: &AllowedDependencies,
    violations: &mut Vec<String>,
) -> Result<()> {
    let manifest_path = dir.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value = toml::from_str(&content)?;

    let this_pkg = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or_else(|| dir.to_str().unwrap_or("unknown"));

    // 1. Check target-independent [dependencies]
    if let Some(deps) = manifest.get("dependencies").and_then(|d| d.as_table()) {
        for dep in deps.keys() {
            if allowed.siblings.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: [dependencies] contains sibling crate dependency '{dep}'"
                ));
            } else if !allowed.target_independent.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: [dependencies] contains unallowed dependency '{dep}'"
                ));
            }
        }
    }

    // 2. Check [target.'cfg(target_arch = "wasm32")'.dependencies]
    if let Some(target_wasm) = manifest
        .get("target")
        .and_then(|t| t.get("cfg(target_arch = \"wasm32\")"))
        .and_then(|c| c.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for dep in target_wasm.keys() {
            if allowed.siblings.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: wasm32 dependencies contains sibling crate '{dep}'"
                ));
            } else if !allowed.wasm32.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: wasm32 dependencies contains unallowed dependency '{dep}'"
                ));
            }
        }
    }

    // 3. Check [target.'cfg(not(target_arch = "wasm32"))'.dependencies]
    if let Some(target_native) = manifest
        .get("target")
        .and_then(|t| t.get("cfg(not(target_arch = \"wasm32\"))"))
        .and_then(|c| c.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for dep in target_native.keys() {
            if allowed.siblings.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: native dependencies contains sibling crate '{dep}'"
                ));
            } else if !allowed.native.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: native dependencies contains unallowed dependency '{dep}'"
                ));
            }
        }
    }

    // 4. Check component metadata dependencies
    if let Some(meta_deps) = manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("component"))
        .and_then(|c| c.get("target"))
        .and_then(|t| t.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for dep in meta_deps.keys() {
            if allowed.siblings.contains(&dep.as_str()) {
                violations.push(format!(
                    "{this_pkg}: component metadata dependencies contains sibling crate '{dep}'"
                ));
            }
        }
    }

    Ok(())
}

fn get_workspace_root() -> PathBuf {
    if Path::new("crates").exists() {
        PathBuf::from(".")
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new(".")).to_path_buf()
    }
}

fn check_roym_deps() -> Result<()> {
    println!("Checking Roym service crate dependency hygiene...");
    let siblings = [
        "syneroym-roym-web",
        "syneroym-roym-conversation",
        "syneroym-roym-profile",
        "syneroym-roym-catalog",
        "syneroym-roym-transaction",
        "syneroym-roym-directory",
    ];

    let workspace_root = get_workspace_root();

    let mut crate_dirs = Vec::new();
    let crates_dir = workspace_root.join("crates");
    if let Ok(entries) = fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with("roym_")
            {
                crate_dirs.push(path);
            }
        }
    }
    crate_dirs.sort();

    let allowed = AllowedDependencies {
        siblings: &siblings,
        target_independent: &[
            "syneroym-app-host",
            "syneroym-roym-core",
            "syneroym-signed-record",
            "serde",
            "serde_json",
            "async-trait",
            "thiserror",
        ],
        wasm32: &["wit-bindgen", "syneroym-wit-interfaces"],
        native: &["syneroym-rpc", "syneroym-app-host-native", "async-trait"],
    };

    let mut violations = Vec::new();

    for dir in &crate_dirs {
        check_crate_manifest_dependencies(dir, &allowed, &mut violations)?;
    }

    if !violations.is_empty() {
        for v in &violations {
            eprintln!("ERROR: {v}");
        }
        bail!("Dependency hygiene check failed with {} violation(s)", violations.len());
    }

    println!("All Roym service crates adhere to dependency hygiene rules.");
    Ok(())
}

struct PerfBenchmarkResults {
    bench_rows: Vec<String>,
    latency_rows: Vec<String>,
    concurrency_rows: Vec<String>,
    soak_rows: Vec<String>,
}

fn run_micro_benchmarks() -> Result<Vec<String>> {
    println!("Running cargo bench...");
    let status = Command::new("cargo")
        .args(["bench", "--workspace"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        println!("cargo bench failed, continuing anyway...");
    }

    let mut bench_rows = Vec::new();
    let criterion_dir = Path::new("target/criterion");
    if criterion_dir.exists() {
        for entry in WalkDir::new(criterion_dir).into_iter().filter_map(|e| e.ok()) {
            if entry.path().ends_with("new/estimates.json")
                && let Ok(content) = fs::read_to_string(entry.path())
                && let Ok(json) = serde_json::from_str::<Value>(&content)
                && let Some(mean) =
                    json.get("mean").and_then(|m| m.get("point_estimate")).and_then(|p| p.as_f64())
            {
                let parts: Vec<_> = entry.path().iter().collect();
                if parts.len() >= 4 {
                    let name = parts[parts.len() - 3].to_string_lossy().to_string();
                    bench_rows.push(format!("| {} | {:.2} ms |", name, mean / 1_000_000.0));
                }
            }
        }
    }
    Ok(bench_rows)
}

fn run_latency_benchmarks() -> Result<Vec<String>> {
    println!("Running syneroym-perf latency...");
    let latency_out = Command::new("cargo")
        .args(["run", "--release", "-p", "syneroym-perf", "--", "latency"])
        .output()?;
    let latency_stdout = String::from_utf8_lossy(&latency_out.stdout);
    let mut latency_rows = Vec::new();
    let mut current_scenario = String::new();
    for line in latency_stdout.lines() {
        if line.starts_with("=== Latency Comparison:") {
            current_scenario =
                line.replace("=== Latency Comparison: ", "").replace(" ===", "").trim().to_string();
        } else if line.starts_with("Via Substrate") {
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 4 {
                latency_rows
                    .push(format!("| {} | {} ms | {} ms |", current_scenario, parts[1], parts[2]));
            }
        }
    }
    Ok(latency_rows)
}

fn run_concurrency_benchmarks(perf_results: &Path) -> Result<Vec<String>> {
    println!("Running syneroym-perf concurrency...");
    Command::new("cargo")
        .args(["run", "--release", "-p", "syneroym-perf", "--", "concurrency"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    let mut concurrency_rows = Vec::new();
    if perf_results.exists() {
        let mut concurrency_files: Vec<_> = fs::read_dir(perf_results)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("concurrency_"))
            .collect();
        concurrency_files.sort_by_key(|a| Reverse(a.metadata().unwrap().modified().unwrap()));
        if let Some(file) = concurrency_files.first()
            && let Ok(content) = fs::read_to_string(file.path())
            && let Ok(json) = serde_json::from_str::<Value>(&content)
            && let Some(summaries) = json.get("summaries").and_then(|s| s.as_array())
        {
            for s in summaries {
                let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown");
                let thr = s.get("throughput_rps").and_then(|t| t.as_f64()).unwrap_or(0.0);
                let err = s.get("error_rate").and_then(|e| e.as_f64()).unwrap_or(0.0) * 100.0;
                let p95 = s.get("latency_p95_ms").and_then(|l| l.as_f64()).unwrap_or(0.0);
                concurrency_rows.push(format!("| {name} | {thr:.1} | {err:.2}% | {p95:.2} ms |"));
            }
        }
    }
    Ok(concurrency_rows)
}

fn run_soak_benchmarks(perf_results: &Path) -> Result<Vec<String>> {
    println!("Running syneroym-perf soak...");
    Command::new("cargo")
        .args(["run", "--release", "-p", "syneroym-perf", "--", "soak"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    let mut soak_rows = Vec::new();
    if perf_results.exists() {
        let mut soak_files: Vec<_> = fs::read_dir(perf_results)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("soak_"))
            .collect();
        soak_files.sort_by_key(|a| Reverse(a.metadata().unwrap().modified().unwrap()));
        if let Some(file) = soak_files.first()
            && let Ok(content) = fs::read_to_string(file.path())
            && let Ok(json) = serde_json::from_str::<Value>(&content)
        {
            let dur = json.get("duration_secs").and_then(|d| d.as_u64()).unwrap_or(0);
            let thr = json.get("rpc_throughput").and_then(|t| t.as_f64()).unwrap_or(0.0);
            let pass = json.get("overall_pass").and_then(|p| p.as_bool()).unwrap_or(false);
            let rss = json.get("rss_peak_mb").and_then(|r| r.as_f64()).unwrap_or(0.0);
            let res = if pass { "✅ PASS" } else { "❌ FAIL" };
            soak_rows.push(format!("| {dur}s | {thr:.1} | {rss:.1} MB | {res} |"));
        }
    }
    Ok(soak_rows)
}

fn append_perf_summary_sections(
    summary_path: &str,
    timestamp: &str,
    commit: &str,
    env_line: &str,
    results: &PerfBenchmarkResults,
) -> Result<()> {
    println!("Updating PERF_SUMMARY.md...");
    let is_new = !Path::new(summary_path).exists();
    let mut file = OpenOptions::new().create(true).append(true).open(summary_path)?;

    if is_new {
        writeln!(file, "# Performance Summary")?;
        writeln!(file, "\nThis file is automatically updated by `cargo xtask perf-summary`.")?;
    }

    writeln!(file, "\n## Run: {timestamp} ({commit})")?;
    writeln!(file, "\n### Environment")?;
    writeln!(file, "| Commit | Timestamp | OS | CPU | Memory |")?;
    writeln!(file, "|--------|-----------|----|-----|--------|")?;
    writeln!(file, "{env_line}")?;

    if !results.bench_rows.is_empty() {
        writeln!(file, "\n### Criterion Micro-Benchmarks")?;
        writeln!(file, "| Benchmark | Mean Time (ms) |")?;
        writeln!(file, "|-----------|----------------|")?;
        for row in &results.bench_rows {
            writeln!(file, "{row}")?;
        }
    }

    if !results.latency_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Latency")?;
        writeln!(file, "| Scenario | p50 | p95 |")?;
        writeln!(file, "|----------|-----|-----|")?;
        for row in &results.latency_rows {
            writeln!(file, "{row}")?;
        }
    }

    if !results.concurrency_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Concurrency")?;
        writeln!(file, "| Scenario | Throughput (rps) | Error Rate | p95 Latency |")?;
        writeln!(file, "|----------|------------------|------------|-------------|")?;
        for row in &results.concurrency_rows {
            writeln!(file, "{row}")?;
        }
    }

    if !results.soak_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Soak")?;
        writeln!(file, "| Duration | Throughput (rps) | Peak RSS | Result |")?;
        writeln!(file, "|----------|------------------|----------|--------|")?;
        for row in &results.soak_rows {
            writeln!(file, "{row}")?;
        }
    }

    println!("Done! Results appended to PERF_SUMMARY.md");
    Ok(())
}

fn perf_summary() -> Result<()> {
    println!("Gathering environment details...");
    let commit = get_git_commit();
    let (os, cpu, mem) = get_sys_info();
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let env_line = format!("| {commit} | {timestamp} | {os} | {cpu} | {mem} |");

    let perf_results = Path::new("tests/perf/results");
    let results = PerfBenchmarkResults {
        bench_rows: run_micro_benchmarks()?,
        latency_rows: run_latency_benchmarks()?,
        concurrency_rows: run_concurrency_benchmarks(perf_results)?,
        soak_rows: run_soak_benchmarks(perf_results)?,
    };

    append_perf_summary_sections("PERF_SUMMARY.md", &timestamp, &commit, &env_line, &results)
}

fn is_test_path(path: &Path) -> bool {
    for component in path.iter() {
        if component == "tests" {
            return true;
        }
    }
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
        && (file_name.starts_with("tests_")
            || file_name.ends_with("_tests.rs")
            || file_name == "tests.rs")
    {
        return true;
    }
    false
}

fn count_production_lines(content: &str) -> usize {
    let mut prod_lines = 0;
    let mut in_test = false;
    let mut test_depth = 0;
    let mut brace_depth = 0;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("#[cfg(test)]") {
            in_test = true;
            test_depth = brace_depth;
        }
        let open_b = line.matches('{').count();
        let close_b = line.matches('}').count();
        if in_test {
            brace_depth = brace_depth + open_b - close_b;
            if (open_b == 0 && trimmed.starts_with("mod ") && trimmed.ends_with(';'))
                || (brace_depth <= test_depth && close_b > 0)
            {
                in_test = false;
            }
        } else {
            brace_depth = brace_depth + open_b - close_b;
            prod_lines += 1;
        }
    }
    prod_lines
}

/// Maximum allowed production source lines (excluding `#[cfg(test)]` blocks).
const MAX_PRODUCTION_LINES: usize = 800;

/// Maximum allowed test file lines.
///
/// This is a ratchet: it moves down only. The intent is to bring it toward the
/// 800-line production limit over time as large test files are decomposed.
const MAX_TEST_LINES: usize = 1800;

fn check_file_lengths() -> Result<()> {
    println!("Checking source file lengths...");
    let workspace_root = get_workspace_root();
    let mut violations = Vec::new();
    let mut prod_checked = 0;
    let mut test_checked = 0;

    for base_dir_name in ["crates", "apps"] {
        let base_dir = workspace_root.join(base_dir_name);
        if !base_dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&base_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("rs")
                || path.file_name().and_then(|n| n.to_str()) == Some("bindings.rs")
            {
                continue;
            }

            let is_test = is_test_path(path);
            if !is_test && !path.iter().any(|c| c == "src") {
                continue;
            }

            let rel_path =
                path.strip_prefix(&workspace_root).unwrap_or(path).to_string_lossy().to_string();
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if is_test {
                test_checked += 1;
                let test_lines = content.lines().count();
                if test_lines > MAX_TEST_LINES {
                    violations.push(format!(
                        "{rel_path}: {test_lines} lines (maximum allowed for test files is \
                         {MAX_TEST_LINES})"
                    ));
                }
            } else {
                prod_checked += 1;
                let prod_lines = count_production_lines(&content);
                if prod_lines > MAX_PRODUCTION_LINES {
                    violations.push(format!(
                        "{rel_path}: {prod_lines} production lines (maximum allowed is \
                         {MAX_PRODUCTION_LINES})"
                    ));
                }
            }
        }
    }

    if !violations.is_empty() {
        for v in &violations {
            eprintln!("ERROR: {v}");
        }
        bail!("File length check failed with {} violation(s)", violations.len());
    }

    println!(
        "All {prod_checked} production files (<= {MAX_PRODUCTION_LINES} lines) and {test_checked} \
         test files (<= {MAX_TEST_LINES} lines) adhere to length limits."
    );
    Ok(())
}

/// Ceiling on exact duplicate code percentage across the workspace.
///
/// This value is a ratchet guard against regrowth, not a target: `cargo-dupes`
/// normalises SQL strings (such as distinct `init_schema` definitions) and
/// detects similar repetitive structure patterns across crates as duplicates.
const MAX_EXACT_DUPLICATE_PERCENT: &str = "9.0";

fn check_duplication() -> Result<()> {
    println!("Checking exact-duplicate code percentage (max {MAX_EXACT_DUPLICATE_PERCENT}%)...");
    let workspace_root = get_workspace_root();
    let status = Command::new("cargo")
        .args([
            "dupes",
            "check",
            "--max-exact-percent",
            MAX_EXACT_DUPLICATE_PERCENT,
            "--exclude",
            "bindings.rs",
            "--exclude",
            "target",
        ])
        .current_dir(&workspace_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to run `cargo dupes check` -- is cargo-dupes installed? (`cargo install \
                 cargo-dupes`): {e}"
            )
        })?;

    if !status.success() {
        bail!("Duplication check failed: exact duplication exceeds {MAX_EXACT_DUPLICATE_PERCENT}%");
    }

    Ok(())
}

/// Ceiling on clippy::too_many_lines suppressions across the workspace.
///
/// This is a ratchet: it moves down only. The intent is to cap existing
/// suppressions and ratchet down as oversized functions are decomposed.
const MAX_TOO_MANY_LINES_SUPPRESSIONS: usize = 82;

fn check_lint_suppressions() -> Result<()> {
    println!(
        "Checking clippy::too_many_lines suppressions (max {MAX_TOO_MANY_LINES_SUPPRESSIONS})..."
    );
    let workspace_root = get_workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "*.rs"])
        .current_dir(&workspace_root)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `git ls-files`: {e}"))?;

    if !output.status.success() {
        bail!("`git ls-files` failed");
    }

    let files_str = String::from_utf8_lossy(&output.stdout);
    let mut suppressions = Vec::new();

    for rel_path in files_str.lines() {
        let path = workspace_root.join(rel_path);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut in_allow = false;
        let mut allow_start_line = 0;
        let mut allow_buf = String::new();

        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if !in_allow {
                if trimmed.starts_with("#[allow(") || trimmed.starts_with("#![allow(") {
                    in_allow = true;
                    allow_start_line = line_no + 1;
                    allow_buf.clear();
                    allow_buf.push_str(trimmed);
                    if trimmed.ends_with(")]") {
                        in_allow = false;
                        if allow_buf.contains("too_many_lines") {
                            suppressions.push((rel_path.to_string(), allow_start_line));
                        }
                    }
                }
            } else {
                allow_buf.push(' ');
                allow_buf.push_str(trimmed);
                if trimmed.ends_with(")]") {
                    in_allow = false;
                    if allow_buf.contains("too_many_lines") {
                        suppressions.push((rel_path.to_string(), allow_start_line));
                    }
                }
            }
        }
    }

    let total = suppressions.len();
    println!("Found {total} clippy::too_many_lines suppression(s) across tracked files.");

    if total > MAX_TOO_MANY_LINES_SUPPRESSIONS {
        eprintln!(
            "ERROR: too_many_lines suppressions ({total}) exceed maximum allowed \
             ({MAX_TOO_MANY_LINES_SUPPRESSIONS}):"
        );
        for (file, line) in &suppressions {
            eprintln!("  {file}:{line}");
        }
        bail!(
            "Lint suppressions check failed: {total} exceeds maximum of \
             {MAX_TOO_MANY_LINES_SUPPRESSIONS}"
        );
    }

    println!(
        "All clippy::too_many_lines suppressions are within the limit ({total} <= \
         {MAX_TOO_MANY_LINES_SUPPRESSIONS})."
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("check-roym-deps") => check_roym_deps(),
        Some("check-file-lengths") => check_file_lengths(),
        Some("check-lint-suppressions") => check_lint_suppressions(),
        Some("check-duplication") => check_duplication(),
        Some("perf-summary") | None => perf_summary(),
        Some(other) => bail!("Unknown xtask command: {other}"),
    }
}
