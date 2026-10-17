use super::*;

#[test]
fn public_key_round_trips_through_its_text_form() {
    let key = MailboxSecretKey::generate().unwrap().public_key();
    let text = key.to_string();
    assert_eq!(text.len(), 52);
    assert_eq!(text.parse::<MailboxPublicKey>().unwrap(), key);
}

#[test]
fn secret_key_round_trips_and_derives_the_same_public_key() {
    let key = MailboxSecretKey::generate().unwrap();
    let again = MailboxSecretKey::from_z32(&key.to_z32()).unwrap();
    assert_eq!(again.seed(), key.seed());
    assert_eq!(again.public_key(), key.public_key());
}

#[test]
fn a_public_key_of_the_wrong_length_is_refused() {
    let short = z32::encode(&[1u8; 31]);
    let long = z32::encode(&[1u8; 33]);
    assert_eq!(short.parse::<MailboxPublicKey>(), Err(KeyParseError::WrongLength(31)));
    assert_eq!(long.parse::<MailboxPublicKey>(), Err(KeyParseError::WrongLength(33)));
    assert_eq!(MailboxSecretKey::from_z32(&short).unwrap_err(), KeyParseError::WrongLength(31));
    assert_eq!("not z32 text!".parse::<MailboxPublicKey>(), Err(KeyParseError::NotZ32));
}

#[test]
fn secret_key_debug_does_not_show_the_seed() {
    let key = MailboxSecretKey::generate().unwrap();
    let debug = format!("{key:?}");
    assert!(!debug.contains(key.to_z32().as_str()));
    assert!(debug.contains(&key.public_key().to_string()));
}

#[test]
fn public_key_serializes_as_a_z32_string() {
    let key = MailboxSecretKey::generate().unwrap().public_key();
    let json = serde_json::to_string(&key).unwrap();
    assert_eq!(json, format!("\"{key}\""));
    assert_eq!(serde_json::from_str::<MailboxPublicKey>(&json).unwrap(), key);
    assert!(serde_json::from_str::<MailboxPublicKey>("\"abc\"").is_err());
}

#[test]
fn a_mailbox_key_has_one_text_form() {
    let key = MailboxSecretKey::generate().unwrap().public_key();
    let text = key.to_string();
    let bytes = z32::decode(text.as_bytes()).unwrap();
    // The last of the 52 characters holds 1 bit of the key and 4 spare bits.
    // Another last character can change that 1 bit, and then it is the text
    // of a different key. Take one that decodes to the same 32 bytes.
    let alphabet = "ybndrfg8ejkmcpqxot1uwisza345h769";
    let head = &text[..text.len() - 1];
    let other_spelling = alphabet
        .chars()
        .map(|c| format!("{head}{c}"))
        .find(|candidate| {
            candidate != &text
                && z32::decode(candidate.as_bytes()).ok().as_deref() == Some(bytes.as_slice())
        })
        .unwrap();
    assert_eq!(other_spelling.parse::<MailboxPublicKey>(), Err(KeyParseError::NotCanonical));
    assert_eq!(
        MailboxSecretKey::from_z32(&other_spelling).unwrap_err(),
        KeyParseError::NotCanonical
    );
    // A text with the last character missing reads as zero bits.
    assert_eq!(text[..51].parse::<MailboxPublicKey>(), Err(KeyParseError::NotCanonical));
}
