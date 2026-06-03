use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Barrier, Mutex};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::orchestrator::TestEnvironment;
use crate::reporter::{
    ScenarioResultSummary, get_substrate_metrics, print_concurrency_summary,
    save_concurrency_results,
};

pub async fn run_scenario() -> Result<()> {
    let mut env = TestEnvironment::new().await?;
    env.start_substrate().await?;

    let component_path =
        "test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm";
    let wasm_bytes = fs::read(component_path).context(
        "Failed to read compiled test WASM component. Ensure it has been built successfully.",
    )?;

    let app_identity = syneroym_identity::Identity::generate().unwrap();
    let app_service_id = syneroym_identity::substrate::derive_did_key(&app_identity.public_key());

    let registry_url = "http://127.0.0.1:7961".to_string();
    let mut orchestrator_client =
        syneroym_sdk::SyneroymClient::new(env.substrate_did.clone(), registry_url.clone());
    orchestrator_client.wait_for_ready(Duration::from_secs(10)).await?;

    // Deploy WASM Greeter service
    orchestrator_client
        .deploy_wasm(
            app_service_id.clone(),
            vec!["syneroym-test:greeter/greet@0.1.0".to_string()],
            wasm_bytes,
            None,
        )
        .await?;

    // Register in Registry
    let http_client = reqwest::Client::new();
    let substrate_info = orchestrator_client.lookup().await?;
    let mechanisms = substrate_info.info.mechanisms;

    let info = syneroym_core::community_registry::EndpointInfo {
        service_id: app_service_id.clone(),
        substrate_id: env.substrate_did.clone(),
        endpoint_type: syneroym_core::community_registry::EndpointType::Service,
        nickname: Some("wasm-concurrency".to_string()),
        mechanisms,
        is_private: false,
        ttl: None,
    };
    let info_value = serde_json::to_value(&info).unwrap();
    let canonical_value = syneroym_identity::substrate::canonicalize_json_value(&info_value);
    let canonical_string = serde_json::to_string(&canonical_value).unwrap();
    let _signature = app_identity.sign(canonical_string.as_bytes());

    let signed_info = syneroym_core::community_registry::SignedEndpointInfo {
        info,
        pkarr_packet_hex: "mock-hex".to_string(),
    };

    let res =
        http_client.post(format!("{}/register", registry_url)).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    // Connect Client to the service
    let mut app_client =
        syneroym_sdk::SyneroymClient::new(app_service_id.clone(), registry_url.clone());
    app_client.connect().await?;

    let shared_client = Arc::new(app_client);
    let interface_name = "syneroym-test:greeter/greet@0.1.0";
    let method_name = "greet";

    // --- Start Resource Profiling Task (Category 4) ---
    info!("Starting background resource metrics profiling task");
    let resource_samples = Arc::new(Mutex::new(Vec::new()));
    let stop_profiling = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Capture initial baseline metrics
    let mut baseline_rss = 0.0;
    let mut baseline_cpu = 0.0;
    if let Ok(m) = get_substrate_metrics().await
        && let Some(gauges) = m.get("gauges")
    {
        baseline_rss =
            gauges.get("substrate.system.rss_bytes").and_then(|v| v.as_f64()).unwrap_or(0.0)
                / (1024.0 * 1024.0);
        baseline_cpu =
            gauges.get("substrate.system.cpu_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
    }

    let samples = Arc::clone(&resource_samples);
    let stop = Arc::clone(&stop_profiling);
    let sampler_handle = tokio::spawn(async move {
        let start_time = Instant::now();
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(m) = get_substrate_metrics().await {
                let elapsed = start_time.elapsed().as_secs_f64();
                if let Some(gauges) = m.get("gauges") {
                    let rss = gauges
                        .get("substrate.system.rss_bytes")
                        .and_then(|v| v.as_f64())
                        .map(|v| v / (1024.0 * 1024.0)); // Convert to MB
                    let cpu = gauges.get("substrate.system.cpu_percent").and_then(|v| v.as_f64());
                    let fds = gauges.get("substrate.system.open_fds").and_then(|v| v.as_f64());
                    let wasm_active =
                        gauges.get("substrate.wasm.active_instances").and_then(|v| v.as_f64());

                    let mut s = samples.lock().await;
                    s.push(json!({
                        "elapsed_secs": elapsed,
                        "rss_mb": rss,
                        "cpu_percent": cpu,
                        "open_fds": fds,
                        "wasm_active_instances": wasm_active,
                    }));
                }
            }
            sleep(Duration::from_secs(1)).await;
        }
    });

    // --- Scenario 1: Sustained High Concurrency (100 tasks, 30s) ---
    info!("Running Scenario 1: Sustained High Concurrency");
    let sustained_summary = run_sustained_concurrency(
        Arc::clone(&shared_client),
        interface_name,
        method_name,
        Duration::from_secs(30),
        100,
    )
    .await?;

    // --- Scenario 2: Spike Load (1 to 100 instant spike) ---
    info!("Running Scenario 2: Spike Load");
    let (spike_summary, spike_timeline) = run_spike_load(
        Arc::clone(&shared_client),
        interface_name,
        method_name,
        Duration::from_secs(5),  // Steady state duration
        Duration::from_secs(15), // Spike duration
        100,
    )
    .await?;

    // --- Scenario 3: WASM Pool Exhaustion (20 concurrent calls vs pool limit of 10) ---
    info!("Running Scenario 3: WASM Pool Exhaustion");
    let exhaustion_summary = run_wasm_pool_exhaustion(
        Arc::clone(&shared_client),
        interface_name,
        method_name,
        20, // 20 concurrent tasks
    )
    .await?;

    // --- Scenario 4: Connection Churn (connect/disconnect cycles) ---
    info!("Running Scenario 4: Connection Churn");
    let churn_summary =
        run_connection_churn(registry_url, app_service_id, 10, Duration::from_secs(15)).await?;

    // --- Teardown & Stop Profiling ---
    stop_profiling.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = sampler_handle.await;

    // Collect collected metrics samples
    let samples_lock = resource_samples.lock().await;
    let samples_list = samples_lock.clone();

    let mut peak_rss = baseline_rss;
    let mut peak_cpu = baseline_cpu;
    let mut peak_wasm = 0u64;

    for s in &samples_list {
        if let Some(rss) = s.get("rss_mb").and_then(|v| v.as_f64())
            && rss > peak_rss
        {
            peak_rss = rss;
        }
        if let Some(cpu) = s.get("cpu_percent").and_then(|v| v.as_f64())
            && cpu > peak_cpu
        {
            peak_cpu = cpu;
        }
        if let Some(wasm) = s.get("wasm_active_instances").and_then(|v| v.as_u64())
            && wasm > peak_wasm
        {
            peak_wasm = wasm;
        }
    }

    // Print summary tables
    let summaries = vec![
        sustained_summary.clone(),
        spike_summary.clone(),
        exhaustion_summary.clone(),
        churn_summary.clone(),
    ];
    print_concurrency_summary(
        &summaries,
        baseline_rss,
        peak_rss,
        baseline_cpu,
        peak_cpu,
        peak_wasm,
    );

    // Save JSON results (Category 4 requirement)
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let json_results = json!({
        "timestamp": timestamp,
        "scenarios": {
            "sustained_concurrency": sustained_summary,
            "spike_load": {
                "summary": spike_summary,
                "timeline": spike_timeline
            },
            "wasm_pool_exhaustion": exhaustion_summary,
            "connection_churn": churn_summary
        },
        "resource_profiling": {
            "baseline": {
                "rss_mb": baseline_rss,
                "cpu_percent": baseline_cpu
            },
            "peak": {
                "rss_mb": peak_rss,
                "cpu_percent": peak_cpu,
                "wasm_active_instances": peak_wasm
            },
            "timeline": samples_list
        }
    });

    let saved_path = save_concurrency_results(&json_results, &timestamp)?;
    info!("Detailed concurrency benchmark results written successfully to: {}", saved_path);

    env.teardown().await;
    Ok(())
}

async fn run_sustained_concurrency(
    client: Arc<syneroym_sdk::SyneroymClient>,
    interface: &'static str,
    method: &'static str,
    duration: Duration,
    concurrency: usize,
) -> Result<ScenarioResultSummary> {
    let start_time = Instant::now();
    let latencies = Arc::new(Mutex::new(Vec::new()));
    let success_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let failure_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut handles = Vec::new();

    for _ in 0..concurrency {
        let client = Arc::clone(&client);
        let latencies = Arc::clone(&latencies);
        let success = Arc::clone(&success_count);
        let failure = Arc::clone(&failure_count);

        let handle = tokio::spawn(async move {
            while start_time.elapsed() < duration {
                let req_start = Instant::now();
                match client.request(interface, method, serde_json::json!(["BenchmarkUser"])).await
                {
                    Ok(_) => {
                        success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let elapsed_us = req_start.elapsed().as_micros() as u64;
                        let mut l = latencies.lock().await;
                        l.push(elapsed_us);
                    }
                    Err(e) => {
                        failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        warn!("Sustained load request failed: {:?}", e);
                    }
                }
                tokio::task::yield_now().await;
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    let actual_duration = start_time.elapsed().as_secs_f64();
    let s_count = success_count.load(std::sync::atomic::Ordering::Relaxed);
    let f_count = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    let total = s_count + f_count;

    let mut l = latencies.lock().await;
    l.sort_unstable();

    let p50 = if l.is_empty() { 0.0 } else { l[l.len() / 2] as f64 / 1000.0 };
    let p95 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.95) as usize] as f64 / 1000.0 };
    let p99 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.99) as usize] as f64 / 1000.0 };

    Ok(ScenarioResultSummary {
        name: "Sustained Concurrency".to_string(),
        duration_secs: actual_duration,
        total_requests: total,
        successful_requests: s_count,
        failed_requests: f_count,
        error_rate: if total == 0 { 0.0 } else { f_count as f64 / total as f64 },
        throughput_rps: total as f64 / actual_duration,
        latency_p50_ms: p50,
        latency_p95_ms: p95,
        latency_p99_ms: p99,
    })
}

async fn run_spike_load(
    client: Arc<syneroym_sdk::SyneroymClient>,
    interface: &'static str,
    method: &'static str,
    steady_duration: Duration,
    spike_duration: Duration,
    spike_concurrency: usize,
) -> Result<(ScenarioResultSummary, Vec<serde_json::Value>)> {
    let start_time = Instant::now();
    let timeline = Arc::new(Mutex::new(Vec::new()));
    let success_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let failure_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let latencies = Arc::new(Mutex::new(Vec::new()));

    // 1. Start steady state worker (1 task)
    let s_client = Arc::clone(&client);
    let s_latencies = Arc::clone(&latencies);
    let s_success = Arc::clone(&success_count);
    let s_failure = Arc::clone(&failure_count);
    let timeline_clone = Arc::clone(&timeline);

    let steady_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        while start_time.elapsed() < (steady_duration + spike_duration) {
            interval.tick().await;
            let req_start = Instant::now();
            let elapsed_secs = start_time.elapsed().as_secs_f64();
            match s_client.request(interface, method, serde_json::json!(["BenchmarkUser"])).await {
                Ok(_) => {
                    s_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let elapsed_ms = req_start.elapsed().as_secs_f64() * 1000.0;
                    {
                        let mut l = s_latencies.lock().await;
                        l.push((elapsed_ms * 1000.0) as u64);
                    }
                    let mut t = timeline_clone.lock().await;
                    t.push(json!({
                        "elapsed_secs": elapsed_secs,
                        "latency_ms": elapsed_ms,
                        "status": "success",
                        "phase": if elapsed_secs < steady_duration.as_secs_f64() { "steady" } else { "spike" }
                    }));
                }
                Err(e) => {
                    s_failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let mut t = timeline_clone.lock().await;
                    t.push(json!({
                        "elapsed_secs": elapsed_secs,
                        "status": "error",
                        "error": format!("{:?}", e),
                        "phase": if elapsed_secs < steady_duration.as_secs_f64() { "steady" } else { "spike" }
                    }));
                }
            }
        }
    });

    // 2. Wait until steady state finishes, then instantly spawn spike load
    sleep(steady_duration).await;
    info!("Spike Event Triggered: scaling concurrency instantly from 1 to {}", spike_concurrency);

    let mut spike_handles = Vec::new();
    for _ in 0..(spike_concurrency - 1) {
        let s_client = Arc::clone(&client);
        let s_latencies = Arc::clone(&latencies);
        let s_success = Arc::clone(&success_count);
        let s_failure = Arc::clone(&failure_count);
        let timeline_clone = Arc::clone(&timeline);

        let handle = tokio::spawn(async move {
            let limit = steady_duration + spike_duration;
            while start_time.elapsed() < limit {
                let req_start = Instant::now();
                let elapsed_secs = start_time.elapsed().as_secs_f64();
                match s_client
                    .request(interface, method, serde_json::json!(["BenchmarkUser"]))
                    .await
                {
                    Ok(_) => {
                        s_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let elapsed_ms = req_start.elapsed().as_secs_f64() * 1000.0;
                        {
                            let mut l = s_latencies.lock().await;
                            l.push((elapsed_ms * 1000.0) as u64);
                        }
                        let mut t = timeline_clone.lock().await;
                        t.push(json!({
                            "elapsed_secs": elapsed_secs,
                            "latency_ms": elapsed_ms,
                            "status": "success",
                            "phase": "spike"
                        }));
                    }
                    Err(e) => {
                        s_failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut t = timeline_clone.lock().await;
                        t.push(json!({
                            "elapsed_secs": elapsed_secs,
                            "status": "error",
                            "error": format!("{:?}", e),
                            "phase": "spike"
                        }));
                    }
                }
                tokio::task::yield_now().await;
            }
        });
        spike_handles.push(handle);
    }

    let _ = steady_handle.await;
    for h in spike_handles {
        let _ = h.await;
    }

    let actual_duration = start_time.elapsed().as_secs_f64();
    let s_count = success_count.load(std::sync::atomic::Ordering::Relaxed);
    let f_count = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    let total = s_count + f_count;

    let mut l = latencies.lock().await;
    l.sort_unstable();

    let p50 = if l.is_empty() { 0.0 } else { l[l.len() / 2] as f64 / 1000.0 };
    let p95 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.95) as usize] as f64 / 1000.0 };
    let p99 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.99) as usize] as f64 / 1000.0 };

    let timeline_list = timeline.lock().await.clone();

    Ok((
        ScenarioResultSummary {
            name: "Spike Load".to_string(),
            duration_secs: actual_duration,
            total_requests: total,
            successful_requests: s_count,
            failed_requests: f_count,
            error_rate: if total == 0 { 0.0 } else { f_count as f64 / total as f64 },
            throughput_rps: total as f64 / actual_duration,
            latency_p50_ms: p50,
            latency_p95_ms: p95,
            latency_p99_ms: p99,
        },
        timeline_list,
    ))
}

async fn run_wasm_pool_exhaustion(
    client: Arc<syneroym_sdk::SyneroymClient>,
    interface: &'static str,
    method: &'static str,
    concurrency: usize,
) -> Result<ScenarioResultSummary> {
    let barrier = Arc::new(Barrier::new(concurrency));
    let latencies = Arc::new(Mutex::new(Vec::new()));
    let success_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let failure_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut handles = Vec::new();

    let start_time = Instant::now();

    for _ in 0..concurrency {
        let client = Arc::clone(&client);
        let barrier = Arc::clone(&barrier);
        let latencies = Arc::clone(&latencies);
        let success = Arc::clone(&success_count);
        let failure = Arc::clone(&failure_count);

        let handle = tokio::spawn(async move {
            // Synchronize tasks to make requests at the exact same instant
            barrier.wait().await;
            let req_start = Instant::now();
            match client.request(interface, method, serde_json::json!(["BenchmarkUser"])).await {
                Ok(_) => {
                    success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let elapsed_us = req_start.elapsed().as_micros() as u64;
                    let mut l = latencies.lock().await;
                    l.push(elapsed_us);
                }
                Err(e) => {
                    failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    warn!("WASM Pool Exhaustion request failed gracefully: {:?}", e);
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    let actual_duration = start_time.elapsed().as_secs_f64();
    let s_count = success_count.load(std::sync::atomic::Ordering::Relaxed);
    let f_count = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    let total = s_count + f_count;

    let mut l = latencies.lock().await;
    l.sort_unstable();

    let p50 = if l.is_empty() { 0.0 } else { l[l.len() / 2] as f64 / 1000.0 };
    let p95 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.95) as usize] as f64 / 1000.0 };
    let p99 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.99) as usize] as f64 / 1000.0 };

    Ok(ScenarioResultSummary {
        name: "WASM Pool Exhaustion".to_string(),
        duration_secs: actual_duration,
        total_requests: total,
        successful_requests: s_count,
        failed_requests: f_count,
        error_rate: if total == 0 { 0.0 } else { f_count as f64 / total as f64 },
        throughput_rps: total as f64 / actual_duration,
        latency_p50_ms: p50,
        latency_p95_ms: p95,
        latency_p99_ms: p99,
    })
}

async fn run_connection_churn(
    registry_url: String,
    service_id: String,
    concurrency: usize,
    duration: Duration,
) -> Result<ScenarioResultSummary> {
    let start_time = Instant::now();
    let latencies = Arc::new(Mutex::new(Vec::new()));
    let success_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let failure_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut handles = Vec::new();

    for _ in 0..concurrency {
        let service_id = service_id.clone();
        let registry_url = registry_url.clone();
        let latencies = Arc::clone(&latencies);
        let success = Arc::clone(&success_count);
        let failure = Arc::clone(&failure_count);

        let handle = tokio::spawn(async move {
            while start_time.elapsed() < duration {
                let cycle_start = Instant::now();
                let mut churn_client =
                    syneroym_sdk::SyneroymClient::new(service_id.clone(), registry_url.clone());

                let run_cycle = async {
                    churn_client.connect().await?;
                    let _ = churn_client
                        .request(
                            "syneroym-test:greeter/greet@0.1.0",
                            "greet",
                            serde_json::json!(["BenchmarkUser"]),
                        )
                        .await?;
                    churn_client.shutdown().await?;
                    anyhow::Ok(())
                };

                match run_cycle.await {
                    Ok(_) => {
                        success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let elapsed_us = cycle_start.elapsed().as_micros() as u64;
                        let mut l = latencies.lock().await;
                        l.push(elapsed_us);
                    }
                    Err(e) => {
                        failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        warn!("Connection churn cycle failed: {:?}", e);
                    }
                }
                sleep(Duration::from_millis(50)).await;
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    let actual_duration = start_time.elapsed().as_secs_f64();
    let s_count = success_count.load(std::sync::atomic::Ordering::Relaxed);
    let f_count = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    let total = s_count + f_count;

    let mut l = latencies.lock().await;
    l.sort_unstable();

    let p50 = if l.is_empty() { 0.0 } else { l[l.len() / 2] as f64 / 1000.0 };
    let p95 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.95) as usize] as f64 / 1000.0 };
    let p99 = if l.is_empty() { 0.0 } else { l[(l.len() as f64 * 0.99) as usize] as f64 / 1000.0 };

    Ok(ScenarioResultSummary {
        name: "Connection Churn".to_string(),
        duration_secs: actual_duration,
        total_requests: total,
        successful_requests: s_count,
        failed_requests: f_count,
        error_rate: if total == 0 { 0.0 } else { f_count as f64 / total as f64 },
        throughput_rps: total as f64 / actual_duration,
        latency_p50_ms: p50,
        latency_p95_ms: p95,
        latency_p99_ms: p99,
    })
}
