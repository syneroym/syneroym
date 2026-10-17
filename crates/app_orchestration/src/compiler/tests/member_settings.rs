//! The per-service mailbox and certificate lifetime in the compiler.

use semver::Version;

use super::{visibility::planned_service, *};

const HOST: &str = "did:key:h7wy641jeaawpdosg7mwogqromgksnkk3rh7faa93ga5cntowi1duxwe";

async fn compile_service(extra: &str) -> Result<CompiledDeployment> {
    let toml = format!(
        r#"
        id = "syneroym:settings-app"
        version = "1.0.0"

        [services.web]
        service_type = "wasm"
        source = "web.wasm"
        {extra}
        "#
    );
    let manifest = SynAppManifest::from_toml(&toml).unwrap();
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    compile(AppInstanceId::new("root-inst"), &manifest, &catalog).await
}

#[tokio::test]
async fn both_settings_reach_every_member_of_a_replicated_service() {
    let compiled = compile_service(&format!(
        "visibility = \"public\"\nreplicas = 2\nmailbox = \"{HOST}\"\ninstance_cert_expires_hours \
         = 168"
    ))
    .await
    .unwrap();

    let members = &compiled.plans[0].services;
    assert_eq!(members.len(), 2);
    for member in members {
        assert_eq!(member.mailbox, Some(MailboxChoice::Service(ServiceId::new(HOST))));
        assert_eq!(member.instance_cert_expires_hours, Some(168));
    }
}

#[tokio::test]
async fn a_manifest_without_the_settings_compiles_to_a_plan_without_their_keys() {
    let compiled = compile_service("").await.unwrap();

    let service = &compiled.plans[0].services[0];
    assert_eq!(service.mailbox, None);
    assert_eq!(service.instance_cert_expires_hours, None);
    let json = serde_json::to_string(service).unwrap();
    assert!(!json.contains("mailbox"), "{json}");
    assert!(!json.contains("instance_cert_expires_hours"), "{json}");
}

#[tokio::test]
async fn a_mailbox_on_a_service_that_is_not_public_is_refused() {
    for visibility in ["internal", "private"] {
        let err = compile_service(&format!("visibility = \"{visibility}\"\nmailbox = \"{HOST}\""))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("only a 'public' service can have a mailbox"), "{err}");
        assert!(err.contains(visibility), "{err}");
    }
}

#[tokio::test]
async fn an_auto_mailbox_parses_but_is_refused_for_now() {
    let manifest_text = "visibility = \"public\"\nmailbox = \"auto\"";
    let err = compile_service(manifest_text).await.unwrap_err().to_string();
    assert!(err.contains("not supported yet"), "{err}");
}

#[tokio::test]
async fn a_certificate_lifetime_outside_one_to_720_hours_is_refused() {
    for hours in [0, 721] {
        let err = compile_service(&format!("instance_cert_expires_hours = {hours}"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("from 1 to 720"), "{err}");
    }
    for hours in [1, 720] {
        compile_service(&format!("instance_cert_expires_hours = {hours}")).await.unwrap();
    }
}

#[test]
fn the_plan_check_reads_the_plan_so_the_supervisor_gets_it_too() {
    let mut member = planned_service(
        "web",
        Visibility::Private,
        TopologyVisibility::Restricted,
        None,
        BTreeMap::new(),
    );
    member.mailbox = Some(MailboxChoice::Service(ServiceId::new(HOST)));
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("app-inst"),
        blueprint_id: AppBlueprintId::new("syneroym:app"),
        version: Version::new(1, 0, 0),
        services: vec![member],
    };
    let errs = validate_plan_member_settings(&plan).unwrap_err();
    assert_eq!(errs.len(), 1);
}
