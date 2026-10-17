use syneroym_identity::Identity;

use super::*;
use crate::{
    OpenPolicy, open,
    test_support::{CERT_LIFE_SECS, Member, content, seal_between},
};

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

#[test]
fn the_same_master_gives_the_same_mailbox_key() {
    let master = Identity::generate().unwrap();
    let first = MailboxSecretKey::derive_from_master(&master);
    let second = MailboxSecretKey::derive_from_master(&master);
    assert_eq!(first.public_key(), second.public_key());
    assert_eq!(first.seed(), second.seed());
}

#[test]
fn different_masters_give_different_mailbox_keys() {
    let one = MailboxSecretKey::derive_from_master(&Identity::generate().unwrap());
    let other = MailboxSecretKey::derive_from_master(&Identity::generate().unwrap());
    assert_ne!(one.public_key(), other.public_key());
}

#[test]
fn the_derived_key_is_not_the_master_key() {
    let master = Identity::from_bytes(&[7u8; 32]);
    let key = MailboxSecretKey::derive_from_master(&master);
    assert_ne!(*key.seed(), master.to_bytes());
}

#[test]
fn the_derived_key_has_a_fixed_value() {
    // This value pins the derivation. Never change it: a changed value
    // means every deployed member would get a new mailbox key.
    let master = Identity::from_bytes(&[7u8; 32]);
    let key = MailboxSecretKey::derive_from_master(&master).public_key();
    assert_eq!(key.to_string(), "dnsbyd8idm96xbpyjo4b8zgtp14rg77hrnp6pwqs437cprjg1fio");
}

#[test]
fn a_derived_key_opens_what_was_sealed_to_it() {
    let sender = Member::new(CERT_LIFE_SECS);
    let mut receiver = Member::new(CERT_LIFE_SECS);
    receiver.mailbox = MailboxSecretKey::derive_from_master(&receiver.master);
    receiver.mailbox_public = receiver.mailbox.public_key();
    let (sealed, now) = seal_between(&sender, &receiver);
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &sealed.deposit.deposit_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    )
    .unwrap();
    assert_eq!(opened.content, content());
}
