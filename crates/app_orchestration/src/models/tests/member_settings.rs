//! The manifest text form of the per-service mailbox and certificate lifetime.

use super::*;

const HOST: &str = "did:key:h7wy641jeaawpdosg7mwogqromgksnkk3rh7faa93ga5cntowi1duxwe";

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

#[test]
fn a_mailbox_that_is_a_did_prefix_with_garbage_is_refused() {
    // `ServiceId` accepts anything after `did:key:`; a mailbox host must decode.
    for text in ["did:key:", "did:key:nonsense", "did:key:z6MkExample"] {
        assert!(text.parse::<MailboxChoice>().is_err(), "{text}");
    }
}

#[test]
fn a_manifest_with_mailbox_methods_parses() {
    let toml = format!(
        r#"
        id = "syneroym:settings-app"
        version = "1.0.0"

        [services.web]
        service_type = "wasm"
        source = "web.wasm"
        visibility = "public"
        mailbox = "{HOST}"
        mailbox_methods = ["greeter#greet", "api#invoke"]
        "#
    );
    let manifest = SynAppManifest::from_toml(&toml).unwrap();
    let spec = &manifest.services[&LogicalServiceName::new("web")];
    let methods: Vec<String> = spec.mailbox_methods.iter().map(ToString::to_string).collect();
    assert_eq!(methods, ["greeter#greet", "api#invoke"]);
    assert_eq!(spec.mailbox_methods[0].interface(), "greeter");
    assert_eq!(spec.mailbox_methods[0].method(), "greet");
}

#[test]
fn a_mailbox_method_that_is_not_interface_hash_method_is_refused() {
    for text in ["a#b#c", "#m", "i#", "i #m", "i#m\n", "plain", "", "i#m n"] {
        assert!(text.parse::<MailboxMethod>().is_err(), "{text:?}");
    }
    let long = "x".repeat(129);
    assert!(format!("{long}#m").parse::<MailboxMethod>().is_err());
    assert!(format!("i#{long}").parse::<MailboxMethod>().is_err());
    assert!(format!("{}#m", "x".repeat(128)).parse::<MailboxMethod>().is_ok());
}

#[test]
fn a_mailbox_method_round_trips_through_its_text_form() {
    let method: MailboxMethod = "greeter#greet".parse().unwrap();
    let json = serde_json::to_string(&method).unwrap();
    assert_eq!(json, "\"greeter#greet\"");
    assert_eq!(serde_json::from_str::<MailboxMethod>(&json).unwrap(), method);
}
