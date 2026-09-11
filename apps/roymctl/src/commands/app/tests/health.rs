//! Tests for health and alerts CLI parsing and help output.

use std::path::PathBuf;

use clap::CommandFactory;

use super::*;

#[test]
fn test_app_health_command_parsing() {
    let cli = DummyCli::try_parse_from([
        "dummy",
        "health",
        "inst-1",
        "--journal-path",
        "test.db",
        "--watch",
        "5",
        "--strict",
    ])
    .unwrap();

    match cli.command {
        AppCommands::Health { instance_id, journal_path, watch, strict, no_record, .. } => {
            assert_eq!(instance_id, "inst-1");
            assert_eq!(journal_path, PathBuf::from("test.db"));
            assert_eq!(watch, Some(5));
            assert!(strict);
            assert!(!no_record);
        }
        _ => panic!("Expected Health command"),
    }
}

#[test]
fn test_app_alerts_command_parsing() {
    let cli = DummyCli::try_parse_from(["dummy", "alerts", "inst-1", "--all"]).unwrap();

    match cli.command {
        AppCommands::Alerts { instance_id, all, .. } => {
            assert_eq!(instance_id, "inst-1");
            assert!(all);
        }
        _ => panic!("Expected Alerts command"),
    }
}

#[test]
fn health_help_lists_no_record_watch_and_strict() {
    let mut cmd = DummyCli::command();
    let help = cmd
        .get_subcommands_mut()
        .find(|c| c.get_name() == "health")
        .expect("health subcommand")
        .render_help()
        .to_string();
    assert!(help.contains("--no-record"), "{help}");
    assert!(help.contains("--watch"), "{help}");
    assert!(help.contains("--strict"), "{help}");
}
