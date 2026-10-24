use std::{collections::BTreeMap, path::PathBuf};

use syneroym_app_orchestration::{
    AppBlueprintId, AppInstanceId, DeploymentJournal, DeploymentPlan, DeploymentState,
    LocalFilesystemCatalog, LogicalServiceName, MailboxChoice, ServiceSpec, SynAppManifest,
    compile, models::SubstrateAlias,
};
use syneroym_core::curated_list::MailboxEntry;
use syneroym_sdk::deploy::MailboxDirectory;
use tokio::runtime::Builder;

use crate::commands::app::settings::{
    apply_mailbox_overrides, pin_auto_mailboxes, refuse_reserved_mailbox_methods,
    refuse_settings_without_masters,
};

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

#[test]
fn a_manifest_that_lists_a_node_interface_as_a_mailbox_method_is_refused() {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let ok = manifest(&format!("mailbox = \"{HOST}\"\nmailbox_methods = [\"greeter#greet\"]"));
    assert!(refuse_reserved_mailbox_methods(&plans_of(&ok, &catalog)).is_ok());

    let bad =
        manifest(&format!("mailbox = \"{HOST}\"\nmailbox_methods = [\"security#inject-kek\"]"));
    let err = refuse_reserved_mailbox_methods(&plans_of(&bad, &catalog)).unwrap_err().to_string();
    assert!(err.contains("security#inject-kek"), "{err}");
    assert!(err.contains("node or platform interface"), "{err}");
}

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";
const LIST_REGISTRY: &str = "http://registry.example.org:7961";

#[derive(Debug)]
struct ListOnly(Vec<MailboxEntry>);

impl ListOnly {
    fn of(dids: &[&str]) -> Self {
        Self(
            dids.iter()
                .map(|d| MailboxEntry { service_id: (*d).to_string(), operator: "op".to_string() })
                .collect(),
        )
    }
}

#[async_trait::async_trait]
impl MailboxDirectory for ListOnly {
    async fn open_mailboxes(&self) -> anyhow::Result<Vec<MailboxEntry>> {
        Ok(self.0.clone())
    }

    async fn named_mailbox(
        &self,
        _service_id: &str,
    ) -> anyhow::Result<Option<syneroym_app_orchestration::ServiceId>> {
        panic!("the plan holds made-up ids here, so no record may be read");
    }
}

fn pin(
    plan: &mut DeploymentPlan,
    journal: &DeploymentJournal,
    directory: Option<&dyn MailboxDirectory>,
    list_registry: Option<&str>,
    hosts: &BTreeMap<SubstrateAlias, (Option<String>, bool)>,
) -> anyhow::Result<()> {
    Builder::new_current_thread().build().unwrap().block_on(pin_auto_mailboxes(
        plan,
        journal,
        directory,
        list_registry,
        hosts,
    ))
}

fn auto_plan() -> DeploymentPlan {
    plans(&manifest("mailbox = \"auto\"")).remove(0)
}

fn mailbox_of(plan: &DeploymentPlan) -> Option<String> {
    plan.services[0].mailbox.as_ref().map(ToString::to_string)
}

#[test]
fn a_rerun_keeps_the_mailbox_auto_chose_first() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let first_list = ListOnly::of(&[DID_A, DID_B]);
    let mut first = auto_plan();
    pin(&mut first, &journal, Some(&first_list), Some(LIST_REGISTRY), &BTreeMap::new()).unwrap();
    let chosen = mailbox_of(&first).expect("auto was pinned");
    assert!(chosen == DID_A || chosen == DID_B);
    journal.append(&first, DeploymentState::Active).unwrap();

    let other = if chosen == DID_A { DID_B } else { DID_A };
    let changed_list = ListOnly::of(&[other]);
    let mut second = auto_plan();
    pin(&mut second, &journal, Some(&changed_list), Some(LIST_REGISTRY), &BTreeMap::new()).unwrap();

    assert_eq!(mailbox_of(&second), Some(chosen));
}

#[test]
fn auto_with_no_registry_and_no_journal_record_is_refused() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let mut plan = auto_plan();

    let err = pin(&mut plan, &journal, None, None, &BTreeMap::new()).unwrap_err();

    assert!(format!("{err:#}").contains("no registry is given"), "{err:#}");
    assert_eq!(plan.services[0].mailbox, Some(MailboxChoice::Auto));
}

#[test]
fn a_new_choice_for_a_host_on_another_registry_is_refused_and_nothing_is_journaled() {
    let journal = DeploymentJournal::open_in_memory().unwrap();
    let list = ListOnly::of(&[DID_A]);
    let mut plan = auto_plan();
    plan.services[0].substrate = Some(SubstrateAlias::new("edge"));
    let hosts = BTreeMap::from([(
        SubstrateAlias::new("edge"),
        (Some("http://other.example.org:7961".to_string()), false),
    )]);

    let err = pin(&mut plan, &journal, Some(&list), Some(LIST_REGISTRY), &hosts).unwrap_err();

    assert!(err.to_string().contains("other.example.org"), "{err}");
    assert!(journal.get_latest(&plan.app_instance_id).unwrap().is_none());
}
