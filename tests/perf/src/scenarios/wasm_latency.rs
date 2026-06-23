use std::{
    fs,
    time::{Duration, Instant},
};

use anyhow::Result;
use reqwest::Client;
use syneroym_app_sandbox::{AppSandboxEngine, HostState};
use syneroym_core::{
    dht_registry::{EndpointInfo, EndpointType},
    test_constants,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;
use test_constants::GREETER_INTERFACE_NAME;
use wasmtime::{
    Store,
    component::{Component, Val},
};

use crate::{orchestrator::TestEnvironment, reporter::print_latency_comparison};

pub async fn run_scenario() -> Result<()> {
    let mut env = TestEnvironment::new().await?;
    env.start_substrate().await?;

    let component_path = test_constants::greeter_wasm_path();
    let wasm_bytes = fs::read(component_path)?;

    // 1. In-process Baseline
    let engine = AppSandboxEngine::build_wasm_engine(None, None)?;
    let linker = AppSandboxEngine::build_wasm_linker(&engine)?;
    let component = Component::new(&engine, &wasm_bytes)?;
    let expected_result =
        serde_json::json!("Hello, BenchmarkUser! Greetings from greeter::greet::greet");

    let interface_name = GREETER_INTERFACE_NAME;
    let method_name = "greet";

    // Warmup Baseline
    for _ in 0..10 {
        let host_state = HostState::new("test_component".to_string());
        let mut store = Store::new(&engine, host_state);
        let instance = linker.instantiate_async(&mut store, &component).await?;
        let (func, results_len, _item) =
            AppSandboxEngine::get_wasm_func(&mut store, &instance, interface_name, method_name)?;
        let mut wasm_results = vec![Val::Bool(false); results_len];
        func.call_async(
            &mut store,
            &[wasmtime::component::Val::String("BenchmarkUser".to_string())],
            &mut wasm_results,
        )
        .await?;
    }

    // Measure Baseline
    let mut baseline_latencies = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let host_state = HostState::new("test_component".to_string());
        let mut store = Store::new(&engine, host_state);
        let instance = linker.instantiate_async(&mut store, &component).await?;
        let (func, results_len, _item) =
            AppSandboxEngine::get_wasm_func(&mut store, &instance, interface_name, method_name)?;
        let mut wasm_results = vec![Val::Bool(false); results_len];
        func.call_async(
            &mut store,
            &[wasmtime::component::Val::String("BenchmarkUser".to_string())],
            &mut wasm_results,
        )
        .await?;
        baseline_latencies.push(start.elapsed().as_micros() as u64);
    }

    baseline_latencies.sort_unstable();
    let baseline_stats = (
        baseline_latencies[50], // p50
        baseline_latencies[95], // p95
        baseline_latencies[99], // p99
    );

    // 2. Via Substrate
    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());

    let registry_url = "http://127.0.0.1:7961".to_string();
    let mut orchestrator_client =
        SyneroymClient::new(env.substrate_did.clone(), registry_url.clone());
    orchestrator_client.wait_for_ready(Duration::from_secs(10)).await?;

    // Deploy WASM
    orchestrator_client
        .deploy_svc_wasm(app_service_id.clone(), vec![interface_name.to_string()], wasm_bytes, None)
        .await?;

    // We need to register the WASM service in the registry so the client can
    // resolve it
    let http_client = Client::new();
    let substrate_info = orchestrator_client.lookup().await?;
    let mechanisms = substrate_info.info.mechanisms;

    let info = EndpointInfo {
        service_id: app_service_id.clone(),
        substrate_id: env.substrate_did.clone(),
        endpoint_type: EndpointType::Service,
        nickname: Some("wasm-perf".to_string()),
        mechanisms,
        is_private: false,
        ttl: None,
    };
    let signed_info = info.sign(&app_identity).unwrap();

    let res =
        http_client.post(format!("{}/register", registry_url)).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    let mut app_client = SyneroymClient::new(app_service_id.clone(), registry_url.clone());
    app_client.connect().await?;

    // Warmup Via Substrate
    for _ in 0..10 {
        let _ = app_client
            .request(interface_name, method_name, serde_json::json!(["BenchmarkUser"]))
            .await?;
    }

    // Measure Via Substrate
    let mut via_substrate_latencies = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let res = app_client
            .request(interface_name, method_name, serde_json::json!(["BenchmarkUser"]))
            .await?;
        assert_eq!(res.result, expected_result);
        via_substrate_latencies.push(start.elapsed().as_micros() as u64);
    }

    via_substrate_latencies.sort_unstable();
    let via_substrate_stats = (
        via_substrate_latencies[50], // p50
        via_substrate_latencies[95], // p95
        via_substrate_latencies[99], // p99
    );

    print_latency_comparison("WASM Component (Execution)", baseline_stats, via_substrate_stats);

    let _ = app_client.shutdown().await;
    let _ = orchestrator_client.shutdown().await;

    env.teardown().await;
    Ok(())
}
