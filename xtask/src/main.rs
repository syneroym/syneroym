use std::{
    cmp::Reverse,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::Result;
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
    let memory = format!("{:.1} GB", memory_gb);
    (os, cpu, memory)
}

fn main() -> Result<()> {
    println!("Gathering environment details...");
    let commit = get_git_commit();
    let (os, cpu, mem) = get_sys_info();
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let env_line = format!("| {} | {} | {} | {} | {} |", commit, timestamp, os, cpu, mem);

    println!("Running cargo bench...");
    let status = Command::new("cargo")
        .args(["bench", "--workspace"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        println!("cargo bench failed, continuing anyway...");
    }

    // Parse criterion results
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
                // Extract benchmark name from path
                let parts: Vec<_> = entry.path().iter().collect();
                if parts.len() >= 4 {
                    let name = parts[parts.len() - 3].to_string_lossy().to_string();
                    bench_rows.push(format!("| {} | {:.2} ms |", name, mean / 1_000_000.0));
                }
            }
        }
    }

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

    println!("Running syneroym-perf concurrency...");
    Command::new("cargo")
        .args(["run", "--release", "-p", "syneroym-perf", "--", "concurrency"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    // find newest concurrency json
    let mut concurrency_rows = Vec::new();
    let perf_results = Path::new("tests/perf/results");
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
                concurrency_rows
                    .push(format!("| {} | {:.1} | {:.2}% | {:.2} ms |", name, thr, err, p95));
            }
        }
    }

    println!("Running syneroym-perf soak...");
    Command::new("cargo")
        .args(["run", "--release", "-p", "syneroym-perf", "--", "soak"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    // find newest soak json
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
            soak_rows.push(format!("| {}s | {:.1} | {:.1} MB | {} |", dur, thr, rss, res));
        }
    }

    println!("Updating PERF_SUMMARY.md...");
    let summary_path = "PERF_SUMMARY.md";
    let is_new = !Path::new(summary_path).exists();
    let mut file = OpenOptions::new().create(true).append(true).open(summary_path)?;

    if is_new {
        writeln!(file, "# Performance Summary")?;
        writeln!(file, "\nThis file is automatically updated by `cargo xtask perf-summary`.")?;
    }

    writeln!(file, "\n## Run: {} ({})", timestamp, commit)?;
    writeln!(file, "\n### Environment")?;
    writeln!(file, "| Commit | Timestamp | OS | CPU | Memory |")?;
    writeln!(file, "|--------|-----------|----|-----|--------|")?;
    writeln!(file, "{}", env_line)?;

    if !bench_rows.is_empty() {
        writeln!(file, "\n### Criterion Micro-Benchmarks")?;
        writeln!(file, "| Benchmark | Mean Time (ms) |")?;
        writeln!(file, "|-----------|----------------|")?;
        for row in bench_rows {
            writeln!(file, "{}", row)?;
        }
    }

    if !latency_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Latency")?;
        writeln!(file, "| Scenario | p50 | p95 |")?;
        writeln!(file, "|----------|-----|-----|")?;
        for row in latency_rows {
            writeln!(file, "{}", row)?;
        }
    }

    if !concurrency_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Concurrency")?;
        writeln!(file, "| Scenario | Throughput (rps) | Error Rate | p95 Latency |")?;
        writeln!(file, "|----------|------------------|------------|-------------|")?;
        for row in concurrency_rows {
            writeln!(file, "{}", row)?;
        }
    }

    if !soak_rows.is_empty() {
        writeln!(file, "\n### Syneroym Perf: Soak")?;
        writeln!(file, "| Duration | Throughput (rps) | Peak RSS | Result |")?;
        writeln!(file, "|----------|------------------|----------|--------|")?;
        for row in soak_rows {
            writeln!(file, "{}", row)?;
        }
    }

    println!("Done! Results appended to PERF_SUMMARY.md");
    Ok(())
}
