#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The owner's side of a mailbox passport, through the `roymctl` handlers
//! against booted nodes: `svc deploy --mailbox-passport` and
//! `app deploy --mint-masters --mailbox-passport` sign the child token, the
//! substrate stores it, and the receiver's first pass gets a lease at the
//! mailbox.

use std::{fs, path::Path};

use clap::Parser;
use common::{
    mailbox_passport::PassportDesk,
    mailbox_world::{Options, World, world},
    outbox::artifacts,
};
use roymctl::commands::{
    app::{self, AppCommands},
    svc::{self, SvcCommands},
};
use syneroym_core::test_constants;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{PassportId, facts};

mod common;

#[derive(Parser)]
struct SvcCli {
    #[command(subcommand)]
    command: SvcCommands,
}

#[derive(Parser)]
struct AppCli {
    #[command(subcommand)]
    command: AppCommands,
}

struct Setup {
    world: World,
    dir: tempfile::TempDir,
    passport_file: String,
    passport_id: PassportId,
    wasm_file: String,
}

fn did_of(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

/// A world with a controlled mailbox and a directory for `roymctl`: the
/// world's owner is the identity `owner`, who is also the audience of a
/// passport that allows five messages in a queue.
async fn setup() -> Setup {
    let world = world(Options {
        max_queue_messages: Some(2),
        mailbox_controller: true,
        deploy_receiver: false,
        ..Options::standard()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("identities")).unwrap();
    let owner = &world.restart.owner;
    owner.save_to_path(dir.path().join("identities").join("owner.key")).unwrap();
    world
        .receiver
        .master
        .save_to_path(dir.path().join("identities").join("receiver-master.key"))
        .unwrap();

    let desk = PassportDesk::owned_by(owner, owner, world.mailbox_node().did());
    let passport = desk.passport(&[(facts::MAX_MESSAGES, 5)]);
    let passport_file = dir.path().join("passport.json");
    fs::write(&passport_file, serde_json::to_string(&passport).unwrap()).unwrap();
    let (_proxy, greeter) = artifacts().expect("wasm artifacts not built");
    let wasm_file = dir.path().join("greeter.wasm");
    fs::write(&wasm_file, greeter).unwrap();
    Setup {
        passport_id: PassportId::of(&passport).unwrap(),
        passport_file: passport_file.to_string_lossy().to_string(),
        wasm_file: wasm_file.to_string_lossy().to_string(),
        world,
        dir,
    }
}

impl Setup {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The only lease at the mailbox, after the receiver's node has pulled.
    async fn lease_after_a_pass(&self) -> syneroym_passport::wire::LeaseInfo {
        let node = self.world.receiver_node();
        let summary = node.substrate_client.mailbox_pull_now().await.unwrap();
        assert_eq!(summary.blocked, 0, "{summary:?}");
        assert!(summary.registered >= 1, "{summary:?}");
        let mut leases =
            self.world.mailbox_node().substrate_client.mailbox_passport_leases().await.unwrap();
        assert_eq!(leases.len(), 1, "{leases:?}");
        leases.remove(0)
    }
}

#[tokio::test]
async fn svc_deploy_with_the_passport_flag_gets_a_lease() {
    let setup = setup().await;
    let mailbox = setup.world.mailbox_node().did().to_string();
    let greet = format!("{}#greet", test_constants::GREETER_INTERFACE_NAME);
    let argv = [
        "svc",
        "deploy",
        "--svc-id",
        &setup.world.receiver_id(),
        "--interfaces",
        test_constants::GREETER_INTERFACE_NAME,
        "--wasm",
        &setup.wasm_file,
        "--master",
        "receiver-master",
        "--visibility",
        "public",
        "--registry-url",
        setup.world.receiver_node().registry_url(),
        "--mailbox",
        &mailbox,
        "--mailbox-method",
        &greet,
        "--mailbox-passport",
        &setup.passport_file,
    ];
    let cli = SvcCli::try_parse_from(argv).unwrap();
    let node = setup.world.receiver_node();

    svc::handle(
        &cli.command,
        node.registry_url(),
        node.did().to_string(),
        setup.path(),
        Some("owner"),
        None,
    )
    .await
    .expect("svc deploy with a mailbox passport");

    let lease = setup.lease_after_a_pass().await;
    assert_eq!(lease.passport_id, setup.passport_id);
    assert_eq!(lease.subject, setup.world.receiver_id());
    assert_eq!(lease.owner_did, did_of(&setup.world.restart.owner));
    setup.world.teardown().await;
}

#[tokio::test]
async fn app_deploy_with_the_passport_flag_gets_a_lease() {
    let setup = setup().await;
    let mailbox = setup.world.mailbox_node().did().to_string();
    let manifest = setup.path().join("app.toml");
    fs::write(
        &manifest,
        format!(
            "id = \"syneroym:passport-app\"\nversion = \"0.1.0\"\n\n[services.web]\nservice_type \
             = \"wasm\"\nsource = \"{}\"\ninterfaces = [\"{}\"]\nvisibility = \"public\"\nmailbox \
             = \"{mailbox}\"\nmailbox_methods = [\"{}#greet\"]\n",
            setup.wasm_file,
            test_constants::GREETER_INTERFACE_NAME,
            test_constants::GREETER_INTERFACE_NAME,
        ),
    )
    .unwrap();
    let journal = setup.path().join("deployments.db");
    let argv = [
        "app",
        "deploy",
        "inst-1",
        manifest.to_str().unwrap(),
        "--journal-path",
        journal.to_str().unwrap(),
        "--mint-masters",
        "--registry-url",
        setup.world.receiver_node().registry_url(),
        "--mailbox-passport",
        &setup.passport_file,
    ];
    let cli = AppCli::try_parse_from(argv).unwrap();
    let node = setup.world.receiver_node();

    app::handle(
        &cli.command,
        node.registry_url(),
        Some(node.did().to_string()),
        setup.path(),
        Some("owner"),
        None,
    )
    .await
    .expect("app deploy with a mailbox passport");

    let lease = setup.lease_after_a_pass().await;
    assert_eq!(lease.passport_id, setup.passport_id);
    assert_eq!(lease.owner_did, did_of(&setup.world.restart.owner));
    setup.world.teardown().await;
}
