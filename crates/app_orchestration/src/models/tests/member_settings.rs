//! The manifest text form of the per-service mailbox and certificate lifetime.

use super::*;

const HOST: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn a_manifest_with_a_mailbox_and_a_certificate_lifetime_parses() {
    let toml = format!(
        r#"
        id = "syneroym:settings-app"
        version = "1.0.0"

        [services.web]
        service_type = "wasm"
        source = "web.wasm"
        visibility = "public"
        mailbox = "{HOST}"
        instance_cert_expires_hours = 168
        "#
    );
    let manifest = SynAppManifest::from_toml(&toml).unwrap();
    let spec = &manifest.services[&LogicalServiceName::new("web")];
    assert_eq!(spec.mailbox, Some(MailboxChoice::Service(ServiceId::new(HOST))));
    assert_eq!(spec.instance_cert_expires_hours, Some(168));
}

#[test]
fn a_mailbox_choice_round_trips_through_its_text_form() {
    for text in ["auto", HOST] {
        let choice: MailboxChoice = text.parse().unwrap();
        assert_eq!(choice.to_string(), text);
        let json = serde_json::to_string(&choice).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<MailboxChoice>(&json).unwrap(), choice);
    }
    assert_eq!("auto".parse::<MailboxChoice>().unwrap(), MailboxChoice::Auto);
}

#[test]
fn a_mailbox_that_is_neither_auto_nor_a_did_is_refused() {
    for text in ["", "Auto", "not-a-did", "did:web:example.com"] {
        assert!(text.parse::<MailboxChoice>().is_err(), "{text}");
    }
}
