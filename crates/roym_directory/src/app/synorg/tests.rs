use syneroym_roym_core::{area::Area, safety::PublicationLimits};

use super::SynOrgSettingsInput;

#[test]
fn settings_input_matches_settings_fields() {
    let input = SynOrgSettingsInput {
        name: "Test SynOrg".to_string(),
        rules: "Be nice".to_string(),
        area: vec![Area::Circle { lat_e6: 10_000_000, lon_e6: 20_000_000, radius_m: 5000 }],
        categories: vec!["repair".to_string()],
        support_contact: "contact@example.com".to_string(),
        dispute_path: "disputes".to_string(),
        retention_secs: 86400,
        publication_limits: PublicationLimits { window_secs: 3600, max_per_window: 10 },
    };
    let input_json = serde_json::to_value(&input).unwrap();
    let settings = input.into_settings();
    let settings_json = serde_json::to_value(&settings).unwrap();
    assert_eq!(input_json, settings_json);
}
