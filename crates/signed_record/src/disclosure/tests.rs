use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

use super::*;

#[test]
fn rfc9901_digest_vector() {
    let disclosure = "WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0";
    assert_eq!(disclosure_digest(disclosure), "X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0");
}

#[test]
fn rfc9901_disclosure_with_spaces_decodes() {
    let disclosure = "WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0";
    let decoded = decode_disclosure(disclosure).unwrap();
    assert_eq!(decoded.salt, "_26bc4LT-ac6q2KI6cBW5es");
    assert_eq!(decoded.name, "family_name");
    assert_eq!(decoded.value, json!("Möbius"));
    assert_eq!(decoded.digest, "X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0");
}

#[test]
fn our_encoder_output_is_pinned() {
    let mut salt_bytes = [0u8; 16];
    for (i, b) in salt_bytes.iter_mut().enumerate() {
        *b = i as u8;
    }
    let salt = Salt::from_bytes(salt_bytes);
    let name = "scope";
    let value = json!({"lang": "en", "text": "Consulting work"});

    let encoded = encode_disclosure(&salt, name, &value).unwrap();
    // Verify compact JSON with no spaces before checking against pinned value
    let decoded_raw = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).unwrap();
    let as_str = String::from_utf8(decoded_raw).unwrap();
    assert_eq!(
        as_str,
        r#"["AAECAwQFBgcICQoLDA0ODw","scope",{"lang":"en","text":"Consulting work"}]"#
    );

    let expected_encoded = URL_SAFE_NO_PAD.encode(as_str.as_bytes());
    assert_eq!(encoded, expected_encoded);

    let digest = disclosure_digest(&encoded);
    let expected_digest = disclosure_digest(&expected_encoded);
    assert_eq!(digest, expected_digest);
    // Pin both string and digest
    assert_eq!(
        encoded,
        "WyJBQUVDQXdRRkJnY0lDUW9MREEwT0R3Iiwic2NvcGUiLHsibGFuZyI6ImVuIiwidGV4dCI6IkNvbnN1bHRpbmcgd29yayJ9XQ"
    );
    assert_eq!(digest, "sxMabDwBIe3zklm-6dLsvGCxbxHXNMyR5MPdge44mdU");
}

#[test]
fn decoder_refuses_padding_and_non_base64url() {
    let with_padding = "WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0=";
    assert_eq!(decode_disclosure(with_padding), Err(DisclosureError::NotBase64Url));

    let with_invalid_char =
        "WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0!";
    assert_eq!(decode_disclosure(with_invalid_char), Err(DisclosureError::NotBase64Url));
}

#[test]
fn decoder_refuses_two_and_four_elements() {
    let two = URL_SAFE_NO_PAD.encode(r#"["salt","name"]"#);
    assert_eq!(decode_disclosure(&two), Err(DisclosureError::NotThreeElements(2)));

    let four = URL_SAFE_NO_PAD.encode(r#"["salt","name","value","extra"]"#);
    assert_eq!(decode_disclosure(&four), Err(DisclosureError::NotThreeElements(4)));
}

#[test]
fn decoder_refuses_a_non_string_salt_or_name() {
    let non_string_salt = URL_SAFE_NO_PAD.encode(r#"[123,"name","val"]"#);
    assert_eq!(decode_disclosure(&non_string_salt), Err(DisclosureError::SaltNotString));

    let non_string_name = URL_SAFE_NO_PAD.encode(r#"["salt",123,"val"]"#);
    assert_eq!(decode_disclosure(&non_string_name), Err(DisclosureError::NameNotString));
}

#[test]
fn decoder_refuses_a_fractional_number_deep_in_the_value() {
    let fractional = URL_SAFE_NO_PAD.encode(r#"["salt","name",{"items":[1,2.5]}]"#);
    assert_eq!(decode_disclosure(&fractional), Err(DisclosureError::NonIntegerNumber));
}

#[test]
fn seeded_salt_first_output_is_pinned() {
    let s0 = seeded_salt(0, 0);
    let s1 = seeded_salt(7, 3);
    assert_eq!(s0.as_bytes().len(), 16);
    assert_eq!(s1.as_bytes().len(), 16);
    // Pin outputs
    assert_eq!(
        s0.as_bytes(),
        &[55, 71, 8, 255, 247, 113, 157, 213, 151, 158, 200, 117, 213, 108, 210, 40]
    );
    assert_eq!(
        s1.as_bytes(),
        &[162, 220, 173, 120, 213, 155, 193, 217, 38, 22, 76, 177, 65, 228, 101, 89]
    );
}
