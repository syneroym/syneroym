use std::fs;

use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{Identity, substrate};

use super::*;

#[tokio::test]
async fn test_export_and_import_identity_flow() {
    let dir = tempfile::tempdir().unwrap();
    let name = "test-export-id";

    // 1. Create identity
    handle(
        &IdentityCommands::Create { name: name.to_string() },
        "http://localhost:7960",
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    let key_path = dir.path().join("identities").join(format!("{name}.key"));
    let original_id = Identity::load_from_path(&key_path).unwrap();

    // 2. Export identity using the CLI command directly
    let out_file = dir.path().join("backup.json");
    let rec_file = dir.path().join("recovery.key");
    handle(
        &IdentityCommands::Export {
            name: name.to_string(),
            out: out_file.clone(),
            recovery_key_out: Some(rec_file.clone()),
        },
        "http://localhost:7960",
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    assert!(out_file.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&out_file).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600, "exported backup must be mode 0600 on unix");
    }
    let encoded_key = fs::read_to_string(&rec_file).unwrap();

    // 3. Import in a clean directory
    let clean_dir = tempfile::tempdir().unwrap();
    handle(
        &IdentityCommands::Import {
            name: "restored-id".to_string(),
            r#in: out_file,
            recovery_key: encoded_key,
        },
        "http://localhost:7960",
        clean_dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    // 4. Verify restored identity exists and matches original key bytes and DID
    let restored_path = clean_dir.path().join("identities").join("restored-id.key");
    let restored_id = Identity::load_from_path(&restored_path).unwrap();
    assert_eq!(original_id.public_key(), restored_id.public_key());
    assert_eq!(original_id.to_bytes(), restored_id.to_bytes());
    assert_eq!(
        substrate::derive_did_key(&original_id.public_key()),
        substrate::derive_did_key(&restored_id.public_key())
    );
}

#[tokio::test]
async fn test_revoke_key_refuses_non_did_key() {
    let dir = tempfile::tempdir().unwrap();
    handle(
        &IdentityCommands::Create { name: "master".to_string() },
        "http://localhost:7960",
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    let client = RegistryClient::new(false, Some("http://registry.invalid".to_string()));
    let err = revoke_key(&client, dir.path(), "master", "not-a-did-key", false).await.unwrap_err();
    assert!(err.to_string().contains("must be a did:key"), "{err}");
}

#[tokio::test]
async fn test_revoke_key_refuses_master_own_did() {
    let dir = tempfile::tempdir().unwrap();
    handle(
        &IdentityCommands::Create { name: "master".to_string() },
        "http://localhost:7960",
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    let key_path = dir.path().join("identities").join("master.key");
    let master_id = Identity::load_from_path(&key_path).unwrap();
    let master_did = substrate::derive_did_key(&master_id.public_key());

    let client = RegistryClient::new(false, Some("http://registry.invalid".to_string()));
    let err = revoke_key(&client, dir.path(), "master", &master_did, false).await.unwrap_err();
    assert!(err.to_string().contains("names the master itself"), "{err}");
}
