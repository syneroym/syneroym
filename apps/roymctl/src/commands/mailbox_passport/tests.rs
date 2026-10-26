use serde_json::Map;
use syneroym_passport::{MAILBOX_USE, issue_passport};

use super::*;

const LIFE: u64 = 3_600;

fn did(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

fn now() -> u64 {
    syneroym_passport::now_secs()
}

struct Setup {
    dir: tempfile::TempDir,
    operator: Identity,
    owner: Identity,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("identities")).unwrap();
        let owner = Identity::generate().unwrap();
        owner.save_to_path(identity::identity_key_path(dir.path(), "owner")).unwrap();
        Self { dir, operator: Identity::generate().unwrap(), owner }
    }

    fn host(&self) -> String {
        did(&self.operator)
    }

    fn passport_file(&self, name: &str, audience: &Identity) -> PathBuf {
        let passport = issue_passport(
            &self.operator,
            &self.host(),
            &MAILBOX_USE,
            &did(audience),
            Map::new(),
            LIFE,
            now(),
        )
        .unwrap();
        let path = self.dir.path().join(name);
        fs::write(&path, serde_json::to_string(&passport).unwrap()).unwrap();
        path
    }
}

#[test]
fn no_files_means_no_passports_and_needs_no_identity() {
    let setup = Setup::new();
    assert!(load_passports(setup.dir.path(), &[], None, now()).unwrap().is_none());
}

#[test]
fn a_passport_without_as_is_refused() {
    let setup = Setup::new();
    let file = setup.passport_file("p.json", &setup.owner);
    let err = load_passports(setup.dir.path(), &[file], None, now()).unwrap_err();
    assert!(err.to_string().contains("--mailbox-passport needs --as <owner>"), "{err}");
}

#[test]
fn a_passport_for_another_owner_is_refused() {
    let setup = Setup::new();
    let file = setup.passport_file("p.json", &Identity::generate().unwrap());
    let err = load_passports(setup.dir.path(), &[file], Some("owner"), now()).unwrap_err();
    assert!(format!("{err:#}").contains("not to the owner"), "{err:#}");
}

#[test]
fn an_unknown_identity_is_refused() {
    let setup = Setup::new();
    let file = setup.passport_file("p.json", &setup.owner);
    let err = load_passports(setup.dir.path(), &[file], Some("nobody"), now()).unwrap_err();
    assert!(err.to_string().contains("'nobody' not found"), "{err}");
}

#[test]
fn a_passport_for_the_owner_loads() {
    let setup = Setup::new();
    let file = setup.passport_file("p.json", &setup.owner);
    let (owner, passports) =
        load_passports(setup.dir.path(), &[file], Some("owner"), now()).unwrap().unwrap();
    assert_eq!(did(&owner), did(&setup.owner));
    assert_eq!(passports[0].host(), setup.host());
}

#[test]
fn the_child_token_needs_the_passport_mailbox() {
    let setup = Setup::new();
    let file = setup.passport_file("p.json", &setup.owner);
    let loaded = load_passports(setup.dir.path(), &[file], Some("owner"), now()).unwrap().unwrap();
    let member = did(&Identity::generate().unwrap());

    let host = ServiceId::new(setup.host());
    let token = child_token_for_service(&loaded, Some(&host), &member, now()).unwrap();
    assert!(token.is_some());

    let other = ServiceId::new(did(&Identity::generate().unwrap()));
    let err = child_token_for_service(&loaded, Some(&other), &member, now()).unwrap_err();
    assert!(err.to_string().contains("but --mailbox is"), "{err}");
}
