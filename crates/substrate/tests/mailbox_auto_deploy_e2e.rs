#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `roymctl app deploy` and `roymctl svc deploy` with `mailbox = "auto"`
//! against a booted node (ADR-0026 Decision 8): the handlers read the open
//! list of the registry, pin the choice, and keep it when the list changes.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::Parser;
use common::SubstrateNode;
use roymctl::commands::{
    app::{self, AppCommands},
    member_identity,
    svc::{self, SvcCommands},
};
use rustls::crypto::ring;
use syneroym_app_orchestration::{AppInstanceId, DeploymentJournal, MailboxChoice};
use syneroym_core::{
    config::MailboxRole, curated_list::MailboxEntry, dht_registry::RegistryClient,
};
use syneroym_identity::{Identity, substrate};
use tokio::time;

mod common;

/// A node DID that is not any node of the test.
const OTHER_MAILBOX: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";

#[derive(Parser)]
struct AppCli {
    #[command(subcommand)]
    command: AppCommands,
}

#[derive(Parser)]
struct SvcCli {
    #[command(subcommand)]
    command: SvcCommands,
}

struct Fixture {
    node: SubstrateNode,
    dir: tempfile::TempDir,
}

/// A node that is its own registry and lists only `listed` as an open mailbox.
fn list_only(listed: &str) -> impl FnOnce(common::NodeBuilder) -> common::NodeBuilder + use<> {
    let listed = listed.to_string();
    move |builder| {
        builder.configure(move |c| {
            c.roles.mailbox = Some(MailboxRole::default());
            c.roles.community_registry.as_mut().expect("a registry role").mailboxes =
                vec![MailboxEntry { service_id: listed.clone(), operator: "test".into() }];
        })
    }
}

async fn boot() -> Fixture {
    let _ = ring::default_provider().install_default();
    let owner = Identity::generate().unwrap();
    let node = SubstrateNode::builder().owner(&owner).inject_kek().boot().await;
    let did = node.did().to_string();
    let node = node.restart_with(list_only(&did)).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("identities")).unwrap();
    owner.save_to_path(dir.path().join("identities").join("owner.key")).unwrap();
    Fixture { node, dir }
}

impl Fixture {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    async fn app_deploy(&self, args: &[String]) -> anyhow::Result<()> {
        let argv = ["app", "deploy"].map(String::from).into_iter().chain(args.iter().cloned());
        let cli = AppCli::try_parse_from(argv).unwrap();
        app::handle(
            &cli.command,
            self.node.registry_url(),
            Some(self.node.did().to_string()),
            self.path(),
            Some("owner"),
            None,
        )
        .await
    }

    async fn svc_deploy(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut argv = vec!["svc", "deploy"];
        argv.extend_from_slice(args);
        let cli = SvcCli::try_parse_from(argv).unwrap();
        svc::handle(
            &cli.command,
            self.node.registry_url(),
            self.node.did().to_string(),
            self.path(),
            Some("owner"),
            None,
        )
        .await
    }

    fn journal(&self) -> DeploymentJournal {
        DeploymentJournal::open(self.path(), "deployments.db").unwrap()
    }

    /// The registry now lists only `listed`.
    async fn relist(self, listed: &str) -> Self {
        let node = self.node.restart_with(list_only(listed)).await;
        Self { node, ..self }
    }

    fn manifest(&self, port: u16) -> PathBuf {
        let path = self.path().join("app.toml");
        std::fs::write(
            &path,
            format!(
                "id = \"syneroym:auto-deploy-app\"\nversion = \
                 \"0.1.0\"\n\n[services.backend]\nservice_type = \"tcp\"\nsource = \
                 \"127.0.0.1:{port}\"\nvisibility = \"public\"\nmailbox = \"auto\"\n"
            ),
        )
        .unwrap();
        path
    }
}

fn mailbox_of_latest(journal: &DeploymentJournal, instance: &str) -> Option<String> {
    let record = journal.get_latest(&AppInstanceId::new(instance)).unwrap()?;
    match &record.plan.services[0].mailbox {
        Some(MailboxChoice::Service(host)) => Some(host.to_string()),
        _ => None,
    }
}

/// The arguments of `app deploy --mint-masters` for `instance`, with a
/// registry to read the open mailboxes from when `registry` is given.
fn app_args(fx: &Fixture, instance: &str, manifest: &Path, registry: Option<&str>) -> Vec<String> {
    let mut args = vec![
        instance.to_string(),
        manifest.display().to_string(),
        "--journal-path".to_string(),
        fx.path().join("deployments.db").display().to_string(),
        "--mint-masters".to_string(),
    ];
    if let Some(url) = registry {
        args.extend(["--registry-url".to_string(), url.to_string()]);
    }
    args
}

#[tokio::test]
async fn app_deploy_pins_auto_and_a_later_deploy_keeps_it_when_the_list_changes() {
    let fx = boot().await;
    let [port] = common::alloc_ports();
    let manifest = fx.manifest(port);
    let url = fx.node.registry_url().to_string();

    // Without a registry to read the list from, nothing is journaled.
    let err = fx.app_deploy(&app_args(&fx, "auto-refused", &manifest, None)).await.unwrap_err();
    assert!(format!("{err:#}").contains("no registry is given"), "{err:#}");
    assert!(fx.journal().get_latest(&AppInstanceId::new("auto-refused")).unwrap().is_none());

    // The first deploy takes the one listed mailbox, which is this node.
    fx.app_deploy(&app_args(&fx, "auto-inst", &manifest, Some(&url))).await.unwrap();
    let node_did = fx.node.did().to_string();
    assert_eq!(mailbox_of_latest(&fx.journal(), "auto-inst"), Some(node_did.clone()));

    // The list now names another mailbox. The instance keeps its own.
    let fx = fx.relist(OTHER_MAILBOX).await;
    let url = fx.node.registry_url().to_string();
    fx.app_deploy(&app_args(&fx, "auto-inst", &manifest, Some(&url))).await.unwrap();
    assert_eq!(mailbox_of_latest(&fx.journal(), "auto-inst"), Some(node_did));
    fx.node.teardown().await;
}

#[tokio::test]
async fn svc_deploy_pins_auto_and_a_later_deploy_keeps_the_mailbox_its_record_names() {
    let fx = boot().await;
    let [port] = common::alloc_ports();
    let master = member_identity::resolve_or_mint_member_master(fx.path(), "svcmaster").unwrap();
    let svc_id = substrate::derive_did_key(&master.public_key());
    let url = fx.node.registry_url().to_string();
    let tcp = format!("127.0.0.1:{port}");
    let args = [
        "--svc-id",
        &svc_id,
        "--interfaces",
        "default",
        "--tcp",
        &tcp,
        "--master",
        "svcmaster",
        "--visibility",
        "public",
        "--mailbox",
        "auto",
        "--registry-url",
        &url,
    ];

    fx.svc_deploy(&args).await.unwrap();

    let registry = RegistryClient::new(false, Some(url.clone()));
    let first = wait_for_record(&registry, &svc_id, None).await;
    let node_did = fx.node.did().to_string();
    assert_eq!(first.info.mailbox.as_ref().map(|m| m.service_id.clone()), Some(node_did.clone()));

    // The list changes; the record of the service is the only memory of its
    // earlier choice, and the deploy keeps it.
    let fx = fx.relist(OTHER_MAILBOX).await;
    fx.svc_deploy(&args).await.unwrap();
    let registry = RegistryClient::new(false, Some(fx.node.registry_url().to_string()));
    let second = wait_for_record(&registry, &svc_id, Some(first.pkarr_packet_hex.clone())).await;
    assert_eq!(second.info.mailbox.as_ref().map(|m| m.service_id.clone()), Some(node_did));
    fx.node.teardown().await;
}

/// The record of `svc_id` at the registry, once it differs from `previous`.
async fn wait_for_record(
    registry: &RegistryClient,
    svc_id: &str,
    previous: Option<String>,
) -> syneroym_core::dht_registry::SignedEndpointInfo {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(record) = registry.lookup(svc_id, false).await
            && previous.as_ref() != Some(&record.pkarr_packet_hex)
        {
            return record;
        }
        assert!(Instant::now() < deadline, "the record of {svc_id} never changed at the registry");
        time::sleep(Duration::from_millis(300)).await;
    }
}
