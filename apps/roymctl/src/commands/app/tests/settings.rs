use std::path::PathBuf;

use syneroym_app_orchestration::{
    AppBlueprintId, AppInstanceId, DeploymentPlan, LocalFilesystemCatalog, LogicalServiceName,
    ServiceSpec, SynAppManifest, compile,
};
use tokio::runtime::Builder;

use crate::commands::app::settings::{apply_mailbox_overrides, refuse_settings_without_masters};

const HOST: &str = "did:key:h7wy641jeaawpdosg7mwogqromgksnkk3rh7faa93ga5cntowi1duxwe";

fn manifest(extra: &str) -> SynAppManifest {
    SynAppManifest::from_toml(&format!(
        r#"
        id = "syneroym:settings-app"
        version = "1.0.0"

        [services.web]
        service_type = "wasm"
        source = "web.wasm"
        visibility = "public"
        {extra}
        "#
    ))
    .unwrap()
}

fn web(manifest: &SynAppManifest) -> &ServiceSpec {
    &manifest.services[&LogicalServiceName::new("web")]
}

#[test]
fn an_override_for_an_unknown_service_is_refused_with_the_known_names() {
    let mut m = manifest("");
    let err = apply_mailbox_overrides(&mut m, &[("api".to_string(), HOST.parse().unwrap())])
        .unwrap_err()
        .to_string();
    assert!(err.contains("'api'"), "{err}");
    assert!(err.contains("web"), "{err}");
}

fn plans_of(manifest: &SynAppManifest, catalog: &LocalFilesystemCatalog) -> Vec<DeploymentPlan> {
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(compile(AppInstanceId::new("inst-1"), manifest, catalog))
        .unwrap()
        .plans
}

fn plans(manifest: &SynAppManifest) -> Vec<DeploymentPlan> {
    plans_of(manifest, &LocalFilesystemCatalog::new(PathBuf::from(".")))
}

#[test]
fn an_override_reaches_the_compiled_plan() {
    let mut m = manifest("");
    apply_mailbox_overrides(&mut m, &[("web".to_string(), HOST.parse().unwrap())]).unwrap();
    assert_eq!(web(&m).mailbox.as_ref().map(ToString::to_string).as_deref(), Some(HOST));

    let compiled = plans(&m);
    assert_eq!(
        compiled[0].services[0].mailbox.as_ref().map(ToString::to_string).as_deref(),
        Some(HOST)
    );
}

#[test]
fn a_manifest_mailbox_without_mint_masters_is_refused() {
    let compiled = plans(&manifest(&format!("mailbox = \"{HOST}\"")));
    let err = refuse_settings_without_masters(&compiled, false).unwrap_err().to_string();
    assert!(err.contains("--mint-masters"), "{err}");
    assert!(refuse_settings_without_masters(&compiled, true).is_ok());
}

#[test]
fn a_manifest_certificate_lifetime_without_mint_masters_is_refused() {
    let compiled = plans(&manifest("instance_cert_expires_hours = 168"));
    assert!(refuse_settings_without_masters(&compiled, false).is_err());
    assert!(refuse_settings_without_masters(&compiled, true).is_ok());
}

#[test]
fn a_manifest_without_the_settings_needs_no_masters() {
    assert!(refuse_settings_without_masters(&plans(&manifest("")), false).is_ok());
}

/// An app that the manifest spawns can set the values too, and nothing
/// would use them without masters.
#[test]
fn a_spawned_apps_settings_without_mint_masters_are_refused() {
    let root = SynAppManifest::from_toml(
        r#"
        id = "syneroym:root-app"
        version = "1.0.0"

        [services.main]
        service_type = "wasm"
        source = "main.wasm"

        [dependencies.db]
        mode = "spawn"
        blueprint = "syneroym:db-app"
        "#,
    )
    .unwrap();
    let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    catalog.register(
        AppBlueprintId::new("syneroym:db-app"),
        manifest("instance_cert_expires_hours = 168"),
    );
    let compiled = plans_of(&root, &catalog);
    assert_eq!(compiled.len(), 2);

    let err = refuse_settings_without_masters(&compiled, false).unwrap_err().to_string();
    assert!(err.contains("--mint-masters"), "{err}");
    assert!(refuse_settings_without_masters(&compiled, true).is_ok());
}
