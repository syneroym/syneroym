use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::orchestrator::TestEnvironment;
use crate::reporter::{
    SoakSummaryData, get_substrate_metrics, print_soak_summary, save_soak_results,
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ResourceSample {
    elapsed_secs: f64,
    rss_mb: f64,
    cpu_percent: f64,
    open_fds: u64,
    active_tasks: u64,
    connections_active: u64,
    component_cache_size: u64,
}

fn calculate_median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) { (values[mid - 1] + values[mid]) / 2.0 } else { values[mid] }
}

// Calculate the slope of the simple linear regression line (best-fit line) for a sequence of data points
fn calculate_slope(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_xx = 0.0;
    for (i, &y) in values.iter().enumerate() {
        let x = i as f64;
        sum_x += x;
        sum_y += y;
        sum_xy += x * y;
        sum_xx += x * x;
    }
    let denominator = n * sum_xx - sum_x * sum_x;
    if denominator.abs() > 1e-6 { (n * sum_xy - sum_x * sum_y) / denominator } else { 0.0 }
}

pub async fn run_scenario(duration_secs: u64) -> Result<()> {
    info!("Initializing Soak / Endurance Test Environment...");
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

    // Deploy primary WASM Greeter service
    info!("Deploying primary WASM Greeter service...");
    orchestrator_client
        .deploy_wasm(
            app_service_id.clone(),
            vec!["syneroym-test:greeter/greet@0.1.0".to_string()],
            wasm_bytes.clone(),
            None,
        )
        .await?;

    // Register in Registry
    let http_client = reqwest::Client::new();
    let substrate_info = orchestrator_client.lookup().await?;
    let mechanisms = substrate_info.info.mechanisms;

    let _ = orchestrator_client.shutdown().await;

    let info_reg = syneroym_core::community_registry::EndpointInfo {
        service_id: app_service_id.clone(),
        substrate_id: env.substrate_did.clone(),
        endpoint_type: syneroym_core::community_registry::EndpointType::Service,
        nickname: Some("wasm-soak".to_string()),
        mechanisms: mechanisms.clone(),
        is_private: false,
        ttl: None,
    };
    let info_value = serde_json::to_value(&info_reg).unwrap();
    let canonical_value = syneroym_identity::substrate::canonicalize_json_value(&info_value);
    let canonical_string = serde_json::to_string(&canonical_value).unwrap();
    let _signature = app_identity.sign(canonical_string.as_bytes());

    let signed_info = syneroym_core::community_registry::SignedEndpointInfo {
        info: info_reg,
        pkarr_packet_hex: "mock-hex".to_string(),
    };

    let res =
        http_client.post(format!("{}/register", registry_url)).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    // Connect Primary Client to the service
    let mut app_client =
        syneroym_sdk::SyneroymClient::new(app_service_id.clone(), registry_url.clone());
    app_client.connect().await?;

    let shared_client = Arc::new(Mutex::new(app_client));
    let interface_name = "syneroym-test:greeter/greet@0.1.0";
    let method_name = "greet";

    // Grab initial baseline metrics
    let mut baseline_rss = 0.0;
    let mut baseline_cpu = 0.0;
    let mut baseline_fds = 0;
    let mut baseline_tasks = 0;
    let mut baseline_conns = 0;
    let mut baseline_cache = 0;

    if let Ok(m) = get_substrate_metrics().await
        && let Some(gauges) = m.get("gauges")
    {
        baseline_rss =
            gauges.get("substrate.system.rss_bytes").and_then(|v| v.as_f64()).unwrap_or(0.0)
                / (1024.0 * 1024.0);
        baseline_cpu =
            gauges.get("substrate.system.cpu_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
        baseline_fds =
            gauges.get("substrate.system.open_fds").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
        baseline_tasks =
            gauges.get("substrate.tokio.active_tasks").and_then(|v| v.as_f64()).unwrap_or(0.0)
                as u64;
        baseline_conns =
            gauges.get("substrate.connections.active").and_then(|v| v.as_f64()).unwrap_or(0.0)
                as u64;
        baseline_cache = gauges
            .get("substrate.wasm.component_cache_size")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u64;
    }

    info!(
        "Baselines captured: RSS={:.1}MB, CPU={:.1}%, FDs={}, Tasks={}, Connections={}, Cache={}",
        baseline_rss, baseline_cpu, baseline_fds, baseline_tasks, baseline_conns, baseline_cache
    );

    // --- Start Resource Sampling Task ---
    let resource_samples = Arc::new(Mutex::new(Vec::<ResourceSample>::new()));
    let stop_sampling = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let samples_clone = Arc::clone(&resource_samples);
    let stop_clone = Arc::clone(&stop_sampling);
    let sampler_handle = tokio::spawn(async move {
        let start_time = Instant::now();
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(m) = get_substrate_metrics().await
                && let Some(gauges) = m.get("gauges")
            {
                let rss = gauges
                    .get("substrate.system.rss_bytes")
                    .and_then(|v| v.as_f64())
                    .map(|v| v / (1024.0 * 1024.0))
                    .unwrap_or(0.0);
                let cpu = gauges
                    .get("substrate.system.cpu_percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let fds =
                    gauges.get("substrate.system.open_fds").and_then(|v| v.as_f64()).unwrap_or(0.0)
                        as u64;
                let tasks = gauges
                    .get("substrate.tokio.active_tasks")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as u64;
                let conns = gauges
                    .get("substrate.connections.active")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as u64;
                let cache = gauges
                    .get("substrate.wasm.component_cache_size")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as u64;

                let mut s = samples_clone.lock().await;
                s.push(ResourceSample {
                    elapsed_secs: start_time.elapsed().as_secs_f64(),
                    rss_mb: rss,
                    cpu_percent: cpu,
                    open_fds: fds,
                    active_tasks: tasks,
                    connections_active: conns,
                    component_cache_size: cache,
                });
            }
            sleep(Duration::from_secs(5)).await;
        }
    });

    // --- WORKLOAD A: Sustained WASM RPC Load ---
    let rpc_requests = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let rpc_errors = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stop_workloads = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let rpc_req_clone = Arc::clone(&rpc_requests);
    let rpc_err_clone = Arc::clone(&rpc_errors);
    let stop_workloads_clone = Arc::clone(&stop_workloads);
    let client_clone = Arc::clone(&shared_client);

    let workload_a_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        while !stop_workloads_clone.load(std::sync::atomic::Ordering::Relaxed) {
            interval.tick().await;
            let client = client_clone.lock().await;
            rpc_req_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if client
                .request(interface_name, method_name, json!({ "name": "soak-tester" }))
                .await
                .is_err()
            {
                rpc_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });

    // --- WORKLOAD B: Deploy Churn ---
    let deploy_cycles = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let deploy_errors = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let dep_cycles_clone = Arc::clone(&deploy_cycles);
    let dep_err_clone = Arc::clone(&deploy_errors);
    let stop_workloads_clone2 = Arc::clone(&stop_workloads);
    let wasm_bytes_clone = wasm_bytes.clone();
    let registry_url_clone = registry_url.clone();
    let substrate_did_clone = env.substrate_did.clone();
    let mechanisms_clone = mechanisms.clone();

    let deploy_interval_secs = if duration_secs < 60 { 5 } else { 30 };

    let workload_b_handle = tokio::spawn(async move {
        let mut cycle = 0;
        let http_client = reqwest::Client::new();
        while !stop_workloads_clone2.load(std::sync::atomic::Ordering::Relaxed) {
            sleep(Duration::from_secs(deploy_interval_secs)).await;
            if stop_workloads_clone2.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            cycle += 1;
            dep_cycles_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let churn_identity = syneroym_identity::Identity::generate().unwrap();
            let unique_service_id =
                syneroym_identity::substrate::derive_did_key(&churn_identity.public_key());
            info!("Deploy Churn Cycle {}: Deploying {}", cycle, unique_service_id);

            let mut orchestrator_client = syneroym_sdk::SyneroymClient::new(
                substrate_did_clone.clone(),
                registry_url_clone.clone(),
            );

            if let Err(e) = orchestrator_client.connect().await {
                warn!(
                    "Deploy Churn Cycle {} failed to connect orchestrator client: {:?}",
                    cycle, e
                );
                dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            let deploy_result = orchestrator_client
                .deploy_wasm(
                    unique_service_id.clone(),
                    vec!["syneroym-test:greeter/greet@0.1.0".to_string()],
                    wasm_bytes_clone.clone(),
                    None,
                )
                .await;

            match deploy_result {
                Ok(_) => {
                    // Register the churned service in the registry first
                    let info_reg = syneroym_core::community_registry::EndpointInfo {
                        service_id: unique_service_id.clone(),
                        substrate_id: substrate_did_clone.clone(),
                        endpoint_type: syneroym_core::community_registry::EndpointType::Service,
                        nickname: Some(format!("soak-deploy-{}", cycle)),
                        mechanisms: mechanisms_clone.clone(),
                        is_private: false,
                        ttl: None,
                    };
                    let info_value = serde_json::to_value(&info_reg).unwrap();
                    let canonical_value =
                        syneroym_identity::substrate::canonicalize_json_value(&info_value);
                    let canonical_string = serde_json::to_string(&canonical_value).unwrap();
                    let _signature = churn_identity.sign(canonical_string.as_bytes());

                    let signed_info = syneroym_core::community_registry::SignedEndpointInfo {
                        info: info_reg,
                        pkarr_packet_hex: "mock-hex".to_string(),
                    };

                    let reg_res = http_client
                        .post(format!("{}/register", registry_url_clone))
                        .json(&signed_info)
                        .send()
                        .await;

                    if reg_res.is_err() || !reg_res.unwrap().status().is_success() {
                        warn!(
                            "Deploy Churn Cycle {} failed to register unique service in registry",
                            cycle
                        );
                        dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = orchestrator_client.undeploy(unique_service_id.clone()).await;
                        let _ = orchestrator_client.shutdown().await;
                        continue;
                    }

                    // Try one verify request
                    let mut temp_client = syneroym_sdk::SyneroymClient::new(
                        unique_service_id.clone(),
                        registry_url_clone.clone(),
                    );
                    if let Err(e) = temp_client.connect().await {
                        warn!(
                            "Deploy Churn verify connection failed for {}: {:?}",
                            unique_service_id, e
                        );
                        dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        if let Err(e) = temp_client
                            .request(
                                "syneroym-test:greeter/greet@0.1.0",
                                "greet",
                                json!({ "name": "soak-verify" }),
                            )
                            .await
                        {
                            warn!(
                                "Deploy Churn verify request failed for {}: {:?}",
                                unique_service_id, e
                            );
                            dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        let _ = temp_client.shutdown().await;
                    }

                    // Clean up registry entry
                    let _ = http_client
                        .post(format!("{}/deregister", registry_url_clone))
                        .json(&json!({ "service_id": unique_service_id }))
                        .send()
                        .await;

                    // Undeploy the WASM service from the substrate!
                    if let Err(e) = orchestrator_client.undeploy(unique_service_id.clone()).await {
                        warn!(
                            "Deploy Churn Cycle {} failed to undeploy WASM service {}: {:?}",
                            cycle, unique_service_id, e
                        );
                        dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    warn!("Deploy Churn cycle failed to deploy: {:?}", e);
                    dep_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }

            let _ = orchestrator_client.shutdown().await;
        }
    });

    // --- WORKLOAD C: Connection Churn ---
    let conn_cycles = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let conn_errors = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let conn_cycles_clone = Arc::clone(&conn_cycles);
    let conn_err_clone = Arc::clone(&conn_errors);
    let stop_workloads_clone3 = Arc::clone(&stop_workloads);
    let app_service_id_clone = app_service_id.clone();
    let registry_url_clone2 = registry_url.clone();

    let conn_interval_secs = if duration_secs < 60 { 2 } else { 10 };

    let workload_c_handle = tokio::spawn(async move {
        let mut cycle = 0;
        while !stop_workloads_clone3.load(std::sync::atomic::Ordering::Relaxed) {
            sleep(Duration::from_secs(conn_interval_secs)).await;
            if stop_workloads_clone3.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            cycle += 1;
            conn_cycles_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let mut churn_client = syneroym_sdk::SyneroymClient::new(
                app_service_id_clone.clone(),
                registry_url_clone2.clone(),
            );
            if let Err(e) = churn_client.connect().await {
                warn!("Connection Churn cycle {} connect failed: {:?}", cycle, e);
                conn_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            match churn_client
                .request(
                    "syneroym-test:greeter/greet@0.1.0",
                    "greet",
                    json!({ "name": "soak-churn" }),
                )
                .await
            {
                Ok(_) => {
                    if let Err(e) = churn_client.shutdown().await {
                        warn!("Connection Churn cycle {} shutdown failed: {:?}", cycle, e);
                        conn_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    warn!("Connection Churn cycle {} request failed: {:?}", cycle, e);
                    conn_err_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = churn_client.shutdown().await;
                }
            }
        }
    });

    // --- Let workloads run for the specified duration ---
    info!("Endurance workloads started. Running for {} seconds...", duration_secs);
    sleep(Duration::from_secs(duration_secs)).await;

    // --- Tear down workloads and sampler ---
    info!("Sustained testing duration complete. Stopping workloads...");
    stop_workloads.store(true, std::sync::atomic::Ordering::Relaxed);
    stop_sampling.store(true, std::sync::atomic::Ordering::Relaxed);

    let _ = tokio::join!(workload_a_handle, workload_b_handle, workload_c_handle, sampler_handle);

    // Shutdown the primary client to close its Iroh connection and release its sockets/FDs
    let _ = shared_client.lock().await.shutdown().await;

    // --- Wait a few seconds for metrics stabilization ---
    sleep(Duration::from_secs(5)).await;

    // Clean up registry registration
    let _ = http_client
        .post(format!("{}/deregister", registry_url))
        .json(&json!({ "service_id": app_service_id }))
        .send()
        .await;

    // Capture final ending metrics
    let mut ending_rss = 0.0;
    let mut ending_cpu = 0.0;
    let mut ending_fds = 0;
    let mut ending_tasks = 0;
    let mut ending_conns = 0;
    let mut ending_cache = 0;

    if let Ok(m) = get_substrate_metrics().await
        && let Some(gauges) = m.get("gauges")
    {
        ending_rss =
            gauges.get("substrate.system.rss_bytes").and_then(|v| v.as_f64()).unwrap_or(0.0)
                / (1024.0 * 1024.0);
        ending_cpu =
            gauges.get("substrate.system.cpu_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ending_fds =
            gauges.get("substrate.system.open_fds").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
        ending_tasks =
            gauges.get("substrate.tokio.active_tasks").and_then(|v| v.as_f64()).unwrap_or(0.0)
                as u64;
        ending_conns =
            gauges.get("substrate.connections.active").and_then(|v| v.as_f64()).unwrap_or(0.0)
                as u64;
        ending_cache = gauges
            .get("substrate.wasm.component_cache_size")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u64;
    }

    info!(
        "Ending metrics captured: RSS={:.1}MB, CPU={:.1}%, FDs={}, Tasks={}, Connections={}, Cache={}",
        ending_rss, ending_cpu, ending_fds, ending_tasks, ending_conns, ending_cache
    );

    // --- Leak Analysis & Diagnostics ---
    let samples = resource_samples.lock().await;
    let rss_values: Vec<f64> = samples.iter().map(|s| s.rss_mb).collect();
    let fd_values: Vec<u64> = samples.iter().map(|s| s.open_fds).collect();
    let task_values: Vec<u64> = samples.iter().map(|s| s.active_tasks).collect();
    let conn_values: Vec<u64> = samples.iter().map(|s| s.connections_active).collect();

    let rss_peak = rss_values.iter().copied().fold(0.0, f64::max);
    let fd_peak = fd_values.iter().copied().max().unwrap_or(0);
    let task_peak = task_values.iter().copied().max().unwrap_or(0);
    let conn_peak = conn_values.iter().copied().max().unwrap_or(0);

    // Heuristic 1: Memory (RSS) Leak Detection
    let mut rss_stable = true;
    let mut rss_reason = String::new();
    if rss_values.len() >= 6 {
        let chunk_size = rss_values.len() / 3;
        let mut first_third = rss_values[0..chunk_size].to_vec();
        let mut final_third = rss_values[rss_values.len() - chunk_size..].to_vec();

        let first_median = calculate_median(&mut first_third);
        let final_median = calculate_median(&mut final_third);
        let slope = calculate_slope(&rss_values);

        // Leak if final median is > 15% above first median and trend is positive
        if final_median > first_median * 1.15 && slope > 0.001 {
            rss_stable = false;
            rss_reason = format!(
                "Monotonic Memory (RSS) growth detected (Median {}MB -> {}MB, Slope = {:.4})",
                first_median, final_median, slope
            );
        }
    }

    // Heuristic 2: FDs Leak Detection (Ending should be close to baseline, not leaking)
    let fd_stable = ending_fds <= baseline_fds + 25;

    // Heuristic 3: Tokio Tasks Leak Detection (Ending task count stable)
    let task_stable = ending_tasks <= baseline_tasks + 25;

    // Heuristic 4: Connection Leak Detection (Active connections back to baseline)
    let conn_stable = ending_conns <= baseline_conns + 2;

    // Heuristic 5: Component Cache Leak Detection
    let successfully_deployed = deploy_cycles.load(std::sync::atomic::Ordering::Relaxed)
        - deploy_errors.load(std::sync::atomic::Ordering::Relaxed);

    // Expected cache = baseline (likely 1 or similar) + successfully churn-deployed
    let cache_expected = baseline_cache + successfully_deployed;
    let cache_clean = ending_cache <= cache_expected;

    // Compile verdicts
    let mut leak_reason = None;
    let mut overall_pass = true;

    if !rss_stable {
        overall_pass = false;
        leak_reason = Some(rss_reason);
    } else if !fd_stable {
        overall_pass = false;
        leak_reason = Some(format!(
            "Open FDs leak detected (Baseline = {}, Ending = {})",
            baseline_fds, ending_fds
        ));
    } else if !task_stable {
        overall_pass = false;
        leak_reason = Some(format!(
            "Tokio active tasks leak detected (Baseline = {}, Ending = {})",
            baseline_tasks, ending_tasks
        ));
    } else if !conn_stable {
        overall_pass = false;
        leak_reason = Some(format!(
            "Active connections leak detected (Baseline = {}, Ending = {})",
            baseline_conns, ending_conns
        ));
    } else if !cache_clean {
        overall_pass = false;
        leak_reason = Some(format!(
            "WASM component cache size mismatch (Expected <= {}, Ending = {})",
            cache_expected, ending_cache
        ));
    }

    let summary_data = SoakSummaryData {
        duration_secs,
        rpc_requests: rpc_requests.load(std::sync::atomic::Ordering::Relaxed),
        rpc_errors: rpc_errors.load(std::sync::atomic::Ordering::Relaxed),
        rpc_throughput: rpc_requests.load(std::sync::atomic::Ordering::Relaxed) as f64
            / duration_secs as f64,
        deploy_cycles: deploy_cycles.load(std::sync::atomic::Ordering::Relaxed),
        deploy_errors: deploy_errors.load(std::sync::atomic::Ordering::Relaxed),
        conn_cycles: conn_cycles.load(std::sync::atomic::Ordering::Relaxed),
        conn_errors: conn_errors.load(std::sync::atomic::Ordering::Relaxed),
        rss_baseline_mb: baseline_rss,
        rss_peak_mb: rss_peak,
        rss_end_mb: ending_rss,
        rss_stable,
        fd_baseline: baseline_fds,
        fd_peak,
        fd_end: ending_fds,
        fd_stable,
        task_baseline: baseline_tasks,
        task_peak,
        task_end: ending_tasks,
        task_stable,
        conn_baseline: baseline_conns,
        conn_peak,
        conn_end: ending_conns,
        conn_stable,
        cache_expected,
        cache_actual: ending_cache,
        cache_clean,
        overall_pass,
        leak_reason: leak_reason.clone(),
    };

    // Print summary to console
    print_soak_summary(&summary_data);

    // Save JSON results
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let json_val = serde_json::to_value(&summary_data)?;
    let saved_path = save_soak_results(&json_val, &timestamp)?;
    info!("Detailed soak results saved to: {}", saved_path);

    // Shutdown test env explicitly
    env.teardown().await;

    if !overall_pass {
        anyhow::bail!(
            "Soak test failed due to: {}",
            leak_reason.unwrap_or_else(|| "Unknown resource leak".to_string())
        );
    }

    Ok(())
}
