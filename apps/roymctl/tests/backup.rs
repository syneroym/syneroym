//! Integration tests for Roym backup and restore CLI subcommands.

use std::{error::Error, fs};

use assert_cmd::Command;
use predicates::str::contains;
use roymctl::commands::roym::{
    ARCHIVE_INFO, ARCHIVE_VERSION, RESTORE_DATA_SUCCESS_NOTICE, RoymArchive, backup::data_aad_bytes,
};
use serde_json::json;
use syneroym_identity::{Identity, backup, substrate};

#[test]
fn test_restore_data_notice_matches_canonical() {
    assert_eq!(
        RESTORE_DATA_SUCCESS_NOTICE,
        "Your history and records are restored and can be read. Conversations from before the \
         restore cannot continue: this installation has new addresses. Share your new address \
         with the people you talk to, and start new conversations with them."
    );
}

#[test]
fn test_cli_help_for_new_subcommands() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym").arg("backup").arg("--help").assert().success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym").arg("transaction").arg("booking").arg("--help").assert().success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym").arg("transaction").arg("payment").arg("--help").assert().success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym").arg("transaction").arg("fulfilment").arg("--help").assert().success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("quote")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--slot"));

    Ok(())
}

#[test]
fn test_restore_identity_cli_round_trip() -> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;
    let original_identity = Identity::generate()?;
    let master_bytes = original_identity.to_bytes();
    let recovery_key = backup::generate_recovery_key()?;
    let encoded_key = backup::encode_recovery_key(&recovery_key);
    let identity_backup = backup::export(&original_identity, &recovery_key)?;

    let subject_did = substrate::derive_did_key(&original_identity.public_key());
    let now_secs = 1_700_000_000u64;
    let aad = data_aad_bytes(ARCHIVE_VERSION, &subject_did, now_secs);
    let empty_payload = serde_json::to_vec(&json!({ "bundles": {} }))?;
    let sealed_data = backup::seal(&empty_payload, &recovery_key, ARCHIVE_INFO, &aad)?;

    let archive = RoymArchive {
        archive_version: ARCHIVE_VERSION,
        subject_did,
        produced_at_secs: now_secs,
        identity: identity_backup,
        data: sealed_data,
    };

    let archive_path = temp_dir.path().join("archive.json");
    fs::write(&archive_path, serde_json::to_string_pretty(&archive)?)?;

    let restored_key_path = temp_dir.path().join("restored.key");

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("backup")
        .arg("restore-identity")
        .arg("--in")
        .arg(&archive_path)
        .arg("--recovery-key")
        .arg(&encoded_key)
        .arg("--out")
        .arg(&restored_key_path)
        .assert()
        .success()
        .stdout(contains("Identity restored to"));

    let restored_bytes = fs::read(&restored_key_path)?;
    assert_eq!(restored_bytes, master_bytes);

    // Flipped ciphertext byte fails decrypt
    let mut corrupt_archive = archive.clone();
    let mut chars: Vec<char> = corrupt_archive.identity.ciphertext_z32.chars().collect();
    if let Some(first) = chars.first_mut() {
        *first = if *first == 'a' { 'b' } else { 'a' };
    }
    corrupt_archive.identity.ciphertext_z32 = chars.into_iter().collect();

    let corrupt_path = temp_dir.path().join("corrupt_archive.json");
    fs::write(&corrupt_path, serde_json::to_string_pretty(&corrupt_archive)?)?;

    let fail_key_path = temp_dir.path().join("fail.key");
    let mut cmd_corrupt = Command::cargo_bin("roymctl")?;
    cmd_corrupt
        .arg("roym")
        .arg("backup")
        .arg("restore-identity")
        .arg("--in")
        .arg(&corrupt_path)
        .arg("--recovery-key")
        .arg(&encoded_key)
        .arg("--out")
        .arg(&fail_key_path)
        .assert()
        .failure()
        .stderr(contains("could not decrypt"));

    Ok(())
}
