//! Health checks on the manifest surface: serialization and the defaults.

use super::*;

#[test]
fn a_health_check_round_trips_through_toml_and_json() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"
            interfaces = ["http"]

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
            expect_status = 200
            timeout_ms = 1500

            [services.tcpsvc]
            service_type = "tcp"
            source = "unused"
            interfaces = ["main"]

            [services.tcpsvc.health_check.tcp-connect]
            interface = "main"

            [services.wasmsvc]
            service_type = "wasm"
            source = "unused"
            interfaces = ["rpc"]

            [services.wasmsvc.health_check.rpc]
            interface = "rpc"
            method = "ping"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
    assert_eq!(
        backend.config.health_check,
        Some(HealthCheck::HttpGet(HttpProbe {
            interface: InterfaceName::new("http"),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 1500,
        }))
    );

    // This is the round trip that would catch a #[serde(flatten)] regression:
    // an externally-tagged enum nested inside a struct also using `flatten`.
    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
}

#[test]
fn an_absent_health_check_emits_no_key() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "unused"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(identity.config.health_check, None);
    let serialized = manifest.to_toml().unwrap();
    assert!(!serialized.contains("health_check"));
}

#[test]
fn probe_defaults_apply_when_omitted() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
    match backend.config.health_check.as_ref().unwrap() {
        HealthCheck::HttpGet(p) => {
            assert_eq!(p.expect_status, 200);
            assert_eq!(p.timeout_ms, DEFAULT_PROBE_TIMEOUT_MS);
        }
        other => panic!("expected HttpGet, got {other:?}"),
    }
}

#[test]
fn valid_for_pairs_each_kind_with_its_service_types() {
    let tcp = HealthCheck::TcpConnect(TcpProbe {
        interface: InterfaceName::new("main"),
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(tcp.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

    let http = HealthCheck::HttpGet(HttpProbe {
        interface: InterfaceName::new("main"),
        path: "/healthz".to_string(),
        expect_status: 200,
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(http.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

    let rpc = HealthCheck::Rpc(RpcProbe {
        interface: InterfaceName::new("main"),
        method: "ping".to_string(),
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(rpc.valid_for(), &[ServiceType::Wasm]);
}
