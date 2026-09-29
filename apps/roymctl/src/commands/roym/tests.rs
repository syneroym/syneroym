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
