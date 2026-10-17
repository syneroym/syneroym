use super::*;

const HOST: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn a_service_mailbox_splits_on_the_first_equals_sign() {
    let (name, choice) = parse_service_mailbox(&format!("web={HOST}")).unwrap();
    assert_eq!(name, "web");
    assert_eq!(choice.to_string(), HOST);
}

#[test]
fn auto_passes_the_parser() {
    let (_, choice) = parse_service_mailbox("web=auto").unwrap();
    assert_eq!(choice, MailboxChoice::Auto);
}

#[test]
fn a_service_mailbox_without_a_name_or_a_valid_value_is_refused() {
    assert!(parse_service_mailbox(HOST).is_err());
    assert!(parse_service_mailbox(&format!("={HOST}")).is_err());
    assert!(parse_service_mailbox("web=not-a-did").is_err());
}
