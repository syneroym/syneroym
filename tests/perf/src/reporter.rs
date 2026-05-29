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
