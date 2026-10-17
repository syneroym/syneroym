use std::path::PathBuf;

use syneroym_app_orchestration::{
    LocalFilesystemCatalog, LogicalServiceName, ServiceSpec, SynAppManifest, compile,
};
use tokio::runtime::Builder;

use super::*;
use crate::commands::app::settings::{apply_mailbox_overrides, refuse_settings_without_masters};

const HOST: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

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

#[test]
fn an_override_reaches_the_compiled_plan() {
    let mut m = manifest("");
    apply_mailbox_overrides(&mut m, &[("web".to_string(), HOST.parse().unwrap())]).unwrap();
    assert_eq!(web(&m).mailbox.as_ref().map(ToString::to_string).as_deref(), Some(HOST));

    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(compile(AppInstanceId::new("inst-1"), &m, &catalog))
        .unwrap();
    assert_eq!(
        compiled.plans[0].services[0].mailbox.as_ref().map(ToString::to_string).as_deref(),
        Some(HOST)
    );
}

#[test]
fn a_manifest_mailbox_without_mint_masters_is_refused() {
    let m = manifest(&format!("mailbox = \"{HOST}\""));
    let err = refuse_settings_without_masters(&m, false).unwrap_err().to_string();
    assert!(err.contains("--mint-masters"), "{err}");
    assert!(refuse_settings_without_masters(&m, true).is_ok());
}

#[test]
fn a_manifest_certificate_lifetime_without_mint_masters_is_refused() {
    let m = manifest("instance_cert_expires_hours = 168");
    assert!(refuse_settings_without_masters(&m, false).is_err());
    assert!(refuse_settings_without_masters(&m, true).is_ok());
}

#[test]
fn a_manifest_without_the_settings_needs_no_masters() {
    assert!(refuse_settings_without_masters(&manifest(""), false).is_ok());
}
