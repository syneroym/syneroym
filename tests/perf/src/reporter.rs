use std::{fs, path::Path};

use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

pub async fn get_substrate_metrics() -> Result<Value> {
    let client = Client::new();
    // Default metrics endpoint
    let resp = client.get("http://127.0.0.1:7967/metrics").send().await?;
    let json: Value = resp.json().await?;
    Ok(json)
}

pub fn print_latency_comparison(
    scenario: &str,
    baseline: (u64, u64, u64),
    via_substrate: (u64, u64, u64),
) {
    println!("=== Latency Comparison: {} ===", scenario);
    println!("{:<20} | {:<10} | {:<10} | {:<10}", "Path", "p50 (ms)", "p95 (ms)", "p99 (ms)");
    println!("{:-<20}-+-{:-<10}-+-{:-<10}-+-{:-<10}-", "", "", "", "");
    println!(
        "{:<20} | {:<10.2} | {:<10.2} | {:<10.2}",
        "Baseline",
        baseline.0 as f64 / 1_000.0,
        baseline.1 as f64 / 1_000.0,
        baseline.2 as f64 / 1_000.0
    );
    println!(
        "{:<20} | {:<10.2} | {:<10.2} | {:<10.2}",
        "Via Substrate",
        via_substrate.0 as f64 / 1_000.0,
        via_substrate.1 as f64 / 1_000.0,
        via_substrate.2 as f64 / 1_000.0
    );
    println!();
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct ScenarioResultSummary {
    pub name: String,
    pub duration_secs: f64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub error_rate: f64,
    pub throughput_rps: f64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
}

pub fn print_concurrency_summary(
    summaries: &[ScenarioResultSummary],
    baseline_rss_mb: f64,
    peak_rss_mb: f64,
    baseline_cpu: f64,
    peak_cpu: f64,
    peak_wasm_instances: u64,
) {
    println!(
        "\n=========================================================================================="
    );
    println!(
        "                           PHASE 3 CONCURRENCY TEST SUMMARY                              "
    );
    println!(
        "=========================================================================================="
    );
    println!(
        "{:<22} | {:<8} | {:<8} | {:<6} | {:<10} | {:<8} | {:<8} | {:<8}",
        "Scenario",
        "Duration",
        "Requests",
        "Errors",
        "Err Rate",
        "Throughput",
        "p50 (ms)",
        "p95 (ms)"
    );
    println!(
        "{:-<22}-+-{:-<8}-+-{:-<8}-+-{:-<6}-+-{:-<10}-+-{:-<8}-+-{:-<8}-+-{:-<8}",
        "", "", "", "", "", "", "", ""
    );
    for s in summaries {
        println!(
            "{:<22} | {:<7.1}s | {:<8} | {:<6} | {:<9.2}% | {:<10.1} | {:<8.2} | {:<8.2}",
            s.name,
            s.duration_secs,
            s.total_requests,
            s.failed_requests,
            s.error_rate * 100.0,
            s.throughput_rps,
            s.latency_p50_ms,
            s.latency_p95_ms
        );
    }
    println!(
        "------------------------------------------------------------------------------------------"
    );
    println!("Resource Utilization under Stress:");
    println!("  Memory (RSS): Baseline = {:.1} MB, Peak = {:.1} MB", baseline_rss_mb, peak_rss_mb);
    println!("  CPU Usage:    Baseline = {:.1}%, Peak = {:.1}%", baseline_cpu, peak_cpu);
    println!("  WASM sandbox: Peak Active Instances = {}", peak_wasm_instances);
    println!(
        "==========================================================================================\n"
    );
}

pub fn save_concurrency_results(results: &serde_json::Value, timestamp: &str) -> Result<String> {
    let results_dir = Path::new("tests/perf/results");
    if !results_dir.exists() {
        fs::create_dir_all(results_dir)?;
    }
    let filepath = results_dir.join(format!("concurrency_{}.json", timestamp));
    let file_str = filepath.to_string_lossy().to_string();
    let content = serde_json::to_string_pretty(results)?;
    fs::write(&filepath, content)?;
    Ok(file_str)
}

pub fn save_soak_results(results: &serde_json::Value, timestamp: &str) -> Result<String> {
    let results_dir = Path::new("tests/perf/results");
    if !results_dir.exists() {
        fs::create_dir_all(results_dir)?;
    }
    let filepath = results_dir.join(format!("soak_{}.json", timestamp));
    let file_str = filepath.to_string_lossy().to_string();
    let content = serde_json::to_string_pretty(results)?;
    fs::write(&filepath, content)?;
    Ok(file_str)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SoakSummaryData {
    pub duration_secs: u64,
    pub rpc_requests: u64,
    pub rpc_errors: u64,
    pub rpc_throughput: f64,
    pub deploy_cycles: u64,
    pub deploy_errors: u64,
    pub conn_cycles: u64,
    pub conn_errors: u64,
    pub rss_baseline_mb: f64,
    pub rss_peak_mb: f64,
    pub rss_end_mb: f64,
    pub rss_stable: bool,
    pub fd_baseline: u64,
    pub fd_peak: u64,
    pub fd_end: u64,
    pub fd_stable: bool,
    pub task_baseline: u64,
    pub task_peak: u64,
    pub task_end: u64,
    pub task_stable: bool,
    pub conn_baseline: u64,
    pub conn_peak: u64,
    pub conn_end: u64,
    pub conn_stable: bool,
    pub cache_expected: u64,
    pub cache_actual: u64,
    pub cache_clean: bool,
    pub overall_pass: bool,
    pub leak_reason: Option<String>,
}

pub fn print_soak_summary(data: &SoakSummaryData) {
    println!("\n====================================================================");
    println!("                PHASE 4 SOAK TEST SUMMARY (duration: {}s)", data.duration_secs);
    println!("====================================================================");
    println!(
        "{:<24} | {:<8} | {:<6} | {:<8} | {:<10}",
        "Workload", "Requests", "Errors", "Err Rate", "Throughput"
    );
    println!("{:-<24}-+-{:-<8}-+-{:-<6}-+-{:-<8}-+-{:-<10}", "", "", "", "", "");

    let rpc_err_rate = if data.rpc_requests > 0 {
        (data.rpc_errors as f64 / data.rpc_requests as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "{:<24} | {:<8} | {:<6} | {:<7.2}% | {:<10.1} rps",
        "Sustained RPC Load", data.rpc_requests, data.rpc_errors, rpc_err_rate, data.rpc_throughput
    );

    let deploy_err_rate = if data.deploy_cycles > 0 {
        (data.deploy_errors as f64 / data.deploy_cycles as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "{:<24} | {:<8} | {:<6} | {:<7.2}% | {:<10}",
        "Deploy Churn Cycles", data.deploy_cycles, data.deploy_errors, deploy_err_rate, "-"
    );

    let conn_err_rate = if data.conn_cycles > 0 {
        (data.conn_errors as f64 / data.conn_cycles as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "{:<24} | {:<8} | {:<6} | {:<7.2}% | {:<10}",
        "Connection Churn Cycles", data.conn_cycles, data.conn_errors, conn_err_rate, "-"
    );

    println!("--------------------------------------------------------------------");
    println!("Resource Stability Analysis:");

    let status_str = |stable: bool| if stable { "✅ STABLE" } else { "❌ LEAKING" };

    println!(
        "  Memory (RSS):     Baseline = {:.1} MB, Peak = {:.1} MB, End = {:.1} MB  {}",
        data.rss_baseline_mb,
        data.rss_peak_mb,
        data.rss_end_mb,
        status_str(data.rss_stable)
    );
    println!(
        "  Open FDs:         Baseline = {}, Peak = {}, End = {}                  {}",
        data.fd_baseline,
        data.fd_peak,
        data.fd_end,
        status_str(data.fd_stable)
    );
    println!(
        "  Tokio Tasks:      Baseline = {}, Peak = {}, End = {}                  {}",
        data.task_baseline,
        data.task_peak,
        data.task_end,
        status_str(data.task_stable)
    );
    println!(
        "  Connections:      Baseline = {}, Peak = {}, End = {}                  {}",
        data.conn_baseline,
        data.conn_peak,
        data.conn_end,
        status_str(data.conn_stable)
    );
    println!(
        "  Component Cache:  Expected = {}, Actual = {}                          {}",
        data.cache_expected,
        data.cache_actual,
        if data.cache_clean { "✅ CLEAN" } else { "❌ LEAKING" }
    );
    println!("====================================================================");
    if data.overall_pass {
        println!("RESULT: ✅ NO RESOURCE LEAKS DETECTED");
    } else {
        println!(
            "RESULT: ❌ RESOURCE LEAK DETECTED — {}",
            data.leak_reason.as_deref().unwrap_or("Unknown leak detected")
        );
    }
    println!("====================================================================\n");
}
