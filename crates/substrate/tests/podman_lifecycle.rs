#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! Integration tests for the Podman sandbox lifecycle

use std::{process::Command, time::Duration};

use rustls::crypto::ring;
use syneroym_core::{
    config::{ClientGatewayRole, IrohParentConfig, LogTarget, SubstrateConfig},
    dht_registry::EndpointMechanism,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};
use tracing::debug;

const IROH_PORT: u16 = 7984;
const REGISTRY_PORT: u16 = 7981;
const GATEWAY_PORT: u16 = 7980;

struct SubstrateTestContext {
    #[allow(dead_code)]
    config: SubstrateConfig,
    substrate_client: SyneroymClient,
    substrate_service_id: String,
    gateway_port: u16,
    registry_url: String,
    substrate_mechanisms: Vec<EndpointMechanism>,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    temp_dir: TempDir,
}

impl SubstrateTestContext {
    fn gateway_url(&self) -> String {
        format!("http://localhost:{}", self.gateway_port)
    }

    async fn setup(iroh_port: u16, registry_port: u16, gateway_port: u16) -> Self {
        use syneroym_core::config::{
            CoordinatorIrohConfig, CoordinatorRole, PodmanSandboxRole, RolesConfig,
            ServiceRegistryRole,
        };

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let base_path = temp_dir.path();
        let mut config = SubstrateConfig {
            app_local_data_dir: base_path.join("data"),
            app_data_dir: base_path.join("user_data"),
            app_cache_dir: base_path.join("cache"),
            app_log_dir: base_path.join("logs"),
            profile: "full".to_string(),
            ..SubstrateConfig::default()
        };
        config.resolve_paths();
        config.logging.target = LogTarget::Stdout;

        config.roles = RolesConfig {
            coordinator: Some(CoordinatorRole {
                iroh: Some(CoordinatorIrohConfig {
                    enable_relay: true,
                    http_bind_address: format!("0.0.0.0:{iroh_port}"),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            community_registry: Some(ServiceRegistryRole {
                http_bind_address: format!("0.0.0.0:{registry_port}"),
                ..Default::default()
            }),
            client_gateway: Some(ClientGatewayRole { http_port: gateway_port }),
            podman_sandbox: Some(PodmanSandboxRole { podman_path: "podman".to_string() }),
            ..Default::default()
        };

        let registry_url = format!("http://localhost:{registry_port}");
        config.substrate.registry_url = Some(registry_url.clone());
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("Failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("Substrate failed to run");
        });

        let mut substrate_client =
            SyneroymClient::new(substrate_service_id.clone(), registry_url.clone());

        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("Substrate did not become available in time");

        let substrate_info =
            substrate_client.lookup().await.expect("Failed to lookup substrate info from registry");
        let substrate_mechanisms = substrate_info.info.mechanisms;

        Self {
            config,
            substrate_client,
            substrate_service_id,
            gateway_port,
            registry_url,
            substrate_mechanisms,
            shutdown_tx,
            substrate_handle,
            temp_dir,
        }
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

fn has_podman() -> bool {
    Command::new("podman").arg("info").output().map(|o| o.status.success()).unwrap_or(false)
}

#[tokio::test]
async fn test_podman_lifecycle() {
    let _ = ring::default_provider().install_default();

    if !has_podman() {
        println!("Skipping podman lifecycle integration test because podman is not available.");
        return;
    }

    let ctx = SubstrateTestContext::setup(IROH_PORT, REGISTRY_PORT, GATEWAY_PORT).await;

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());

    // We deploy a simple alpine container that starts an HTTP echo or simple server
    // or just nginx to test port mapping.
    // Nginx is small and runs an HTTP server on port 80.
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        ContainerPortMapping, ContainerVolumeFile, ContainerVolumeMapping, DocumentSource,
    };

    let ports = vec![ContainerPortMapping {
        interface_name: "default".to_string(),
        host_port: None, // let podman allocate dynamically
        container_port: 80,
        protocol: "tcp".to_string(),
    }];

    // The capability this whole path exists for: an off-the-shelf image
    // served content that came in the deploy call, with nothing staged on the
    // substrate host beforehand.
    const INDEX_HTML: &str = "<h1>served from the deploy manifest</h1>";

    let volumes = vec![ContainerVolumeMapping {
        host_path: "html".to_string(),
        container_path: "/usr/share/nginx/html".to_string(),
        files: vec![ContainerVolumeFile {
            relative_path: "index.html".to_string(),
            content: DocumentSource::Inline(INDEX_HTML.to_string()),
        }],
    }];

    debug!(">>> Deploying nginx container");
    ctx.substrate_client
        .deploy_container(
            app_service_id.clone(),
            "docker.io/library/nginx:alpine".to_string(),
            ports,
            volumes,
            None,
        )
        .await
        .expect("SDK Deploy container failed");

    // Verify it is listed
    let services = ctx.substrate_client.list_svcs().await.expect("SDK list_services failed");
    assert!(services.iter().any(|s| s.service_id == app_service_id));
    let svc = services.iter().find(|s| s.service_id == app_service_id).unwrap();
    assert_eq!(svc.endpoint_type, "tcp"); // Registered as TcpHostPort

    // Give it a brief moment to warm up
    time::sleep(Duration::from_secs(3)).await;

    // Verify readiness
    ctx.substrate_client
        .request("orchestrator", "readyz", serde_json::json!([app_service_id.clone()]))
        .await
        .expect("readiness check failed");

    // The manifest-supplied file reached the container's filesystem.
    let container_name = app_service_id.replace(':', "_");
    let served = Command::new("podman")
        .args(["exec", &container_name, "cat", "/usr/share/nginx/html/index.html"])
        .output()
        .expect("podman exec failed to run");
    assert!(
        served.status.success(),
        "reading the mounted file failed: {}",
        String::from_utf8_lossy(&served.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&served.stdout).trim(), INDEX_HTML);

    // And the mount is read-only, so the container cannot rewrite config the
    // substrate owns.
    let write_attempt = Command::new("podman")
        .args(["exec", &container_name, "sh", "-c", "touch /usr/share/nginx/html/breakin 2>&1"])
        .output()
        .expect("podman exec failed to run");
    assert!(
        !write_attempt.status.success(),
        "a volume carrying manifest files was writable by the container"
    );

    // Undeploy
    debug!(">>> Undeploying nginx container");
    ctx.substrate_client
        .undeploy(app_service_id.clone())
        .await
        .expect("SDK Undeploy container failed");

    // Verify removed from list
    let services_after = ctx.substrate_client.list_svcs().await.expect("SDK list_services failed");
    assert!(!services_after.iter().any(|s| s.service_id == app_service_id));

    ctx.teardown().await;
}
