//! Tests for the `app resolve` CLI parsing.

use super::*;

#[test]
fn test_app_resolve_command_parsing() {
    let cli =
        DummyCli::try_parse_from(["dummy", "resolve", "did:key:zAppMaster", "backend"]).unwrap();

    match cli.command {
        AppCommands::Resolve { app_did, service_name } => {
            assert_eq!(app_did, "did:key:zAppMaster");
            assert_eq!(service_name, "backend");
        }
        _ => panic!("Expected Resolve command"),
    }
}
