use std::time::{Duration, Instant};

use anyhow::Result;
use reqwest::Client;
use syneroym_core::{
    dht_registry::{EndpointInfo, EndpointType},
    util::short_hash,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;

use crate::{orchestrator::TestEnvironment, reporter::print_latency_comparison};

pub async fn run_scenario() -> Result<()> {
    let mut env = TestEnvironment::new().await?;
    env.start_miniapp(30001).await?;
    env.start_substrate().await?;

    let http_client = Client::builder().build()?;

    let baseline_url = "http://127.0.0.1:30001/";

    // Warmup baseline
    for _ in 0..10 {
        let _ = http_client.get(baseline_url).send().await?;
    }

    // Measure baseline
    let mut baseline_latencies = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let res = http_client.get(baseline_url).send().await?;
        assert!(res.status().is_success());
        baseline_latencies.push(start.elapsed().as_micros() as u64);
    }

    baseline_latencies.sort_unstable();
    let baseline_stats = (
        baseline_latencies[50], // p50
        baseline_latencies[95], // p95
        baseline_latencies[99], // p99
    );

    // Generate an identity for the TCP app
    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());

    // Default ports for dev mode
    let registry_url = "http://127.0.0.1:7961".to_string();
    let gateway_url = "http://127.0.0.1:7960/".to_string();

    // Connect SDK Client to the orchestrator (which is the substrate itself)
    let mut orchestrator_client =
        SyneroymClient::new(env.substrate_did.clone(), registry_url.clone());
    orchestrator_client.wait_for_ready(Duration::from_secs(10)).await?;

    // Deploy the TCP service on the substrate
    orchestrator_client
        .deploy_tcp(
            app_service_id.clone(),
            vec!["default".to_string()],
            "127.0.0.1".to_string(),
            30001,
            None,
        )
        .await?;

    // We need to register it in the registry so the gateway can resolve it
    let substrate_info = orchestrator_client.lookup().await?;
    let mechanisms = substrate_info.info.mechanisms;

    let info = EndpointInfo {
        service_id: app_service_id.clone(),
        substrate_id: env.substrate_did.clone(),
        endpoint_type: EndpointType::Service,
        nickname: Some("tcp-perf".to_string()),
        mechanisms,
        is_private: false,
        ttl: None,
    };
    let signed_info = info.sign(&app_identity).unwrap();

    let res =
        http_client.post(format!("{registry_url}/register")).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    let interface_hash = short_hash("default");
    let pubkeyhash = short_hash(&app_service_id);
    let host_header = format!("tcp-perf-p{pubkeyhash}-i{interface_hash}.localhost");

    // Warmup Via Substrate
    for _ in 0..10 {
        let _ = http_client.get(&gateway_url).header("Host", &host_header).send().await?;
    }

    // Measure Via Substrate
    let mut via_substrate_latencies = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let res = http_client.get(&gateway_url).header("Host", &host_header).send().await?;
        assert!(res.status().is_success());
        via_substrate_latencies.push(start.elapsed().as_micros() as u64);
    }

    via_substrate_latencies.sort_unstable();
    let via_substrate_stats = (
        via_substrate_latencies[50], // p50
        via_substrate_latencies[95], // p95
        via_substrate_latencies[99], // p99
    );

    print_latency_comparison("TCP Proxy (HTTP GET /)", baseline_stats, via_substrate_stats);

    let _ = orchestrator_client.shutdown().await;
    env.teardown().await;
    Ok(())
}
