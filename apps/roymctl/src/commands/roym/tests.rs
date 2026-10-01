#![allow(clippy::cognitive_complexity)]

use clap::Parser;
use serde_json::json;

use super::*;
use crate::commands::roym::directory::{DirectoryCommands, MemberCommands};

/// A minimal `Parser` wrapper so a bare `Subcommand` enum can be exercised
/// with `try_parse_from` the way clap tests any subcommand tree.
#[derive(Parser)]
struct Wrapper<T: clap::Subcommand> {
    #[command(subcommand)]
    command: T,
}

fn parse_directory(args: &[&str]) -> Result<DirectoryCommands, clap::Error> {
    let mut full = vec!["roymctl"];
    full.extend_from_slice(args);
    Wrapper::<DirectoryCommands>::try_parse_from(full).map(|w| w.command)
}

fn parse_member(args: &[&str]) -> Result<MemberCommands, clap::Error> {
    let mut full = vec!["roymctl"];
    full.extend_from_slice(args);
    Wrapper::<MemberCommands>::try_parse_from(full).map(|w| w.command)
}

fn parse_credential(args: &[&str]) -> Result<CredentialCommands, clap::Error> {
    let mut full = vec!["roymctl"];
    full.extend_from_slice(args);
    Wrapper::<CredentialCommands>::try_parse_from(full).map(|w| w.command)
}

fn parse_group(args: &[&str]) -> Result<GroupCommands, clap::Error> {
    let mut full = vec!["roymctl"];
    full.extend_from_slice(args);
    Wrapper::<GroupCommands>::try_parse_from(full).map(|w| w.command)
}

#[test]
fn parse_directory_find() {
    let cmd = parse_directory(&["find", "--category", "cycling", "--limit", "10"]).unwrap();
    assert!(matches!(cmd, DirectoryCommands::Find { .. }));
}

#[test]
fn parse_directory_serve() {
    let cmd = parse_directory(&[
        "serve",
        "--name",
        "Guild",
        "--rules-file",
        "rules.txt",
        "--category",
        "cycling",
        "--support",
        "a@b.c",
        "--dispute",
        "none",
    ])
    .unwrap();
    assert!(matches!(cmd, DirectoryCommands::Serve { .. }));
}

#[test]
fn parse_directory_add_with_issuer() {
    let cmd = parse_directory(&["add", "did:key:zDir", "--issuer", "did:key:zOwner"]).unwrap();
    match cmd {
        DirectoryCommands::Add { did, issuer, .. } => {
            assert_eq!(did, "did:key:zDir");
            assert_eq!(issuer.as_deref(), Some("did:key:zOwner"));
        }
        other => panic!("expected Add, got {other:?}"),
    }
}

#[test]
fn parse_directory_standing() {
    let cmd = parse_directory(&["standing", "--source", "did:key:zDir", "--member", "did:key:zY"])
        .unwrap();
    assert!(matches!(cmd, DirectoryCommands::Standing { .. }));
}

#[test]
fn parse_directory_memberships() {
    let cmd = parse_directory(&["memberships"]).unwrap();
    assert!(matches!(cmd, DirectoryCommands::Memberships { member: None, .. }));
    let cmd = parse_directory(&["memberships", "--member", "did:key:zY"]).unwrap();
    assert!(matches!(cmd, DirectoryCommands::Memberships { member: Some(_), .. }));
}

#[test]
fn parse_member_add() {
    let cmd = parse_member(&["add", "did:key:zY", "--note", "hello"]).unwrap();
    match cmd {
        MemberCommands::Add { did, note, .. } => {
            assert_eq!(did, "did:key:zY");
            assert_eq!(note, "hello");
        }
        other => panic!("expected Add, got {other:?}"),
    }
}

#[test]
fn parse_member_suspend() {
    let cmd = parse_member(&[
        "suspend",
        "--member",
        "did:key:zY",
        "--rule",
        "no-shows",
        "--reason",
        "test",
        "--listing",
        "lst_1",
        "--until-secs",
        "123",
    ])
    .unwrap();
    match cmd {
        MemberCommands::Suspend { member, rule, listing, until_secs, .. } => {
            assert_eq!(member, "did:key:zY");
            assert_eq!(rule, "no-shows");
            assert_eq!(listing.as_deref(), Some("lst_1"));
            assert_eq!(until_secs, Some(123));
        }
        other => panic!("expected Suspend, got {other:?}"),
    }
}

#[test]
fn parse_member_lift() {
    let cmd = parse_member(&["lift", "--decision", "rec_abc", "--reason", "resolved"]).unwrap();
    assert!(matches!(cmd, MemberCommands::Lift { .. }));
}

#[test]
fn parse_member_decisions() {
    let cmd = parse_member(&["decisions"]).unwrap();
    assert!(matches!(cmd, MemberCommands::Decisions { .. }));
}

#[test]
fn parse_credential_issue() {
    let cmd = parse_credential(&[
        "issue",
        "--member",
        "did:key:zY",
        "--category",
        "cycling",
        "--expires-days",
        "30",
    ])
    .unwrap();
    match cmd {
        CredentialCommands::Issue { member, category, expires_days, .. } => {
            assert_eq!(member, "did:key:zY");
            assert_eq!(category, vec!["cycling".to_string()]);
            assert_eq!(expires_days, 30);
        }
        other => panic!("expected Issue, got {other:?}"),
    }
}

#[test]
fn parse_credential_list() {
    let cmd = parse_credential(&["list"]).unwrap();
    assert!(matches!(cmd, CredentialCommands::List { member: None, .. }));
}

#[test]
fn parse_credential_revoke() {
    let cmd = parse_credential(&["revoke", "--credential", "rec_abc", "--reason", "gone"]).unwrap();
    assert!(matches!(cmd, CredentialCommands::Revoke { .. }));
}

fn svc(service_id: &str, interfaces: &[&str]) -> DeployedService {
    serde_json::from_value(json!({
        "service_id": service_id,
        "interfaces": interfaces,
        "endpoint_type": "wasm",
    }))
    .unwrap()
}

fn roym_svcs() -> Vec<DeployedService> {
    vec![
        svc("did:key:zWeb", &["syneroym-roym:web/api@0.1.0"]),
        svc("did:key:zProfile", &["syneroym-roym:profile/api@0.1.0"]),
        svc("did:key:zConv", &["syneroym-roym:conversation/api@0.1.0"]),
        svc("did:key:zOther", &["syneroym:http/incoming-handler@0.2.0"]),
    ]
}

#[test]
fn find_roym_service_matches_by_app_interface() {
    let svcs = roym_svcs();
    assert_eq!(find_roym_service(&svcs, "conversation").unwrap(), "did:key:zConv");
    assert_eq!(find_roym_service(&svcs, "web").unwrap(), "did:key:zWeb");
}

#[test]
fn find_roym_service_errors_when_absent() {
    let err = find_roym_service(&roym_svcs(), "directory").unwrap_err().to_string();
    assert!(err.contains("no Roym 'directory' service is deployed"), "{err}");
}

#[test]
fn find_roym_service_errors_when_ambiguous() {
    let svcs = vec![
        svc("did:key:zConvA", &["syneroym-roym:conversation/api@0.1.0"]),
        svc("did:key:zConvB", &["syneroym-roym:conversation/api@0.1.0"]),
    ];
    let err = find_roym_service(&svcs, "conversation").unwrap_err().to_string();
    assert!(err.contains("2 Roym 'conversation' services"), "{err}");
}

#[test]
fn parse_window_valid_and_invalid() {
    let v = parse_window("100,200").unwrap();
    assert_eq!(v["earliest_secs"], 100);
    assert_eq!(v["latest_secs"], 200);

    assert!(parse_window("200,100").is_err());
    assert!(parse_window("100").is_err());
    assert!(parse_window("100,200,300").is_err());
    assert!(parse_window("abc,200").is_err());
}

#[test]
fn parse_minor_units_handles_various_exponents() {
    assert_eq!(parse_minor_units("12.34", "USD", 2).unwrap(), 1234);
    assert_eq!(parse_minor_units("12.3", "USD", 2).unwrap(), 1230);
    assert_eq!(parse_minor_units("12", "USD", 2).unwrap(), 1200);
    assert_eq!(parse_minor_units("0.05", "USD", 2).unwrap(), 5);
    assert_eq!(parse_minor_units("0", "USD", 2).unwrap(), 0);
    assert!(parse_minor_units("12.345", "USD", 2).is_err());

    assert_eq!(parse_minor_units("1200", "JPY", 0).unwrap(), 1200);
    assert!(parse_minor_units("12.5", "JPY", 0).is_err());

    assert_eq!(parse_minor_units("12.5", "KWD", 3).unwrap(), 12500);
    assert_eq!(parse_minor_units("12.500", "KWD", 3).unwrap(), 12500);
    assert_eq!(parse_minor_units("12.123", "KWD", 3).unwrap(), 12123);
    assert!(parse_minor_units("12.1234", "KWD", 3).is_err());
}

#[test]
fn parse_group_create() {
    let cmd = parse_group(&["create"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Create { name: None, .. }));

    let cmd = parse_group(&["create", "--name", "Community Garden"]).unwrap();
    match cmd {
        GroupCommands::Create { name: Some(n), .. } => assert_eq!(n, "Community Garden"),
        other => panic!("expected Create with name, got {other:?}"),
    }
}

#[test]
fn parse_group_rename() {
    let cmd = parse_group(&["rename", "--group", "conv:g1", "--name", "New Name"]).unwrap();
    match cmd {
        GroupCommands::Rename { group, name, .. } => {
            assert_eq!(group, "conv:g1");
            assert_eq!(name, "New Name");
        }
        other => panic!("expected Rename, got {other:?}"),
    }
}

#[test]
fn parse_group_add_and_mutual_exclusion() {
    let cmd = parse_group(&["add", "--group", "conv:g1", "--address", "did:key:zAddr"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Add { address: Some(_), person_did: None, .. }));

    let cmd =
        parse_group(&["add", "--group", "conv:g1", "--person-did", "did:key:zPerson"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Add { address: None, person_did: Some(_), .. }));

    let err = parse_group(&[
        "add",
        "--group",
        "conv:g1",
        "--address",
        "did:key:zAddr",
        "--person-did",
        "did:key:zPerson",
    ]);
    assert!(err.is_err(), "passing both address and person_did must fail parsing");
}

#[test]
fn parse_group_remove_and_mutual_exclusion() {
    let cmd = parse_group(&["remove", "--group", "conv:g1", "--address", "did:key:zAddr"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Remove { address: Some(_), person_did: None, .. }));

    let cmd =
        parse_group(&["remove", "--group", "conv:g1", "--person-did", "did:key:zPerson"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Remove { address: None, person_did: Some(_), .. }));

    let err = parse_group(&[
        "remove",
        "--group",
        "conv:g1",
        "--address",
        "did:key:zAddr",
        "--person-did",
        "did:key:zPerson",
    ]);
    assert!(err.is_err(), "passing both address and person_did must fail parsing");
}

#[test]
fn parse_group_list() {
    let cmd = parse_group(&["list"]).unwrap();
    assert!(matches!(cmd, GroupCommands::List { include_hidden: false, .. }));

    let cmd = parse_group(&["list", "--include-hidden"]).unwrap();
    assert!(matches!(cmd, GroupCommands::List { include_hidden: true, .. }));
}

#[test]
fn parse_group_info() {
    let cmd = parse_group(&["info", "--group", "conv:g1"]).unwrap();
    match cmd {
        GroupCommands::Info { group, .. } => assert_eq!(group, "conv:g1"),
        other => panic!("expected Info, got {other:?}"),
    }
}

#[test]
fn parse_group_send() {
    let cmd = parse_group(&["send", "--group", "conv:g1", "--body", "hello members"]).unwrap();
    match cmd {
        GroupCommands::Send { group, body, .. } => {
            assert_eq!(group, "conv:g1");
            assert_eq!(body, "hello members");
        }
        other => panic!("expected Send, got {other:?}"),
    }
}

#[test]
fn parse_group_history() {
    let cmd = parse_group(&["history", "--group", "conv:g1"]).unwrap();
    assert!(matches!(cmd, GroupCommands::History { limit: None, .. }));

    let cmd = parse_group(&["history", "--group", "conv:g1", "--limit", "50"]).unwrap();
    match cmd {
        GroupCommands::History { limit: Some(50), .. } => {}
        other => panic!("expected History with limit 50, got {other:?}"),
    }
}

#[test]
fn parse_group_sync_check_hide_unhide() {
    let cmd = parse_group(&["sync", "--group", "conv:g1"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Sync { .. }));

    let cmd = parse_group(&["check", "--group", "conv:g1"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Check { .. }));

    let cmd = parse_group(&["hide", "--group", "conv:g1"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Hide { .. }));

    let cmd = parse_group(&["unhide", "--group", "conv:g1"]).unwrap();
    assert!(matches!(cmd, GroupCommands::Unhide { .. }));
}
