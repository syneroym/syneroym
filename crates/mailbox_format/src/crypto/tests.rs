use super::*;

const SEED: [u8; 32] = [0x42; 32];

#[test]
fn sealed_bytes_open_with_the_matching_key() {
    let public = public_key_from_seed(&SEED);
    let sealed = seal_to(&public, b"hello").unwrap();
    assert_eq!(sealed.len(), b"hello".len() + SEAL_OVERHEAD);
    assert_eq!(open_with(&SEED, &sealed).unwrap(), b"hello");
}

#[test]
fn sealed_bytes_do_not_open_with_another_key() {
    let sealed = seal_to(&public_key_from_seed(&SEED), b"hello").unwrap();
    assert_eq!(open_with(&[0x43; 32], &sealed), Err(CryptoError::Undecryptable));
}

#[test]
fn a_changed_byte_in_the_key_part_the_body_or_the_tag_fails() {
    let sealed = seal_to(&public_key_from_seed(&SEED), b"some longer plaintext").unwrap();
    for index in [0, 40, sealed.len() - 1] {
        let mut changed = sealed.clone();
        changed[index] ^= 0x01;
        assert_eq!(open_with(&SEED, &changed), Err(CryptoError::Undecryptable), "index {index}");
    }
}

#[test]
fn two_seals_of_the_same_plaintext_differ() {
    let public = public_key_from_seed(&SEED);
    assert_ne!(seal_to(&public, b"same").unwrap(), seal_to(&public, b"same").unwrap());
}

#[test]
fn input_shorter_than_the_overhead_is_refused() {
    for len in [0, 1, SEAL_OVERHEAD - 1] {
        assert_eq!(open_with(&SEED, &vec![0u8; len]), Err(CryptoError::TooShort));
    }
}

#[test]
fn a_stored_sealed_vector_still_opens() {
    // Made once with the final code. It fails if the suite, the `info` tag
    // or the key derivation changes: then no stored message would open.
    const SEALED_HEX: &str = "11bc081f023a2cc99ca2dfd0dd364c3fa07d522dc8a9960bd5c90c4ef6e34a5101ec118f5b260388fd825d1b74ecfe4b93b359be334c93684170bb48fb95ded39198ace23b87ed";
    let sealed = hex::decode(SEALED_HEX).unwrap();
    assert_eq!(open_with(&SEED, &sealed).unwrap(), b"syneroym mailbox vector");
}

#[test]
fn a_fixed_seed_gives_a_fixed_public_key() {
    // A change here would make every published mailbox key wrong.
    assert_eq!(
        hex::encode(public_key_from_seed(&SEED)),
        "ae3bf1cd87c2d2ed25af4a1a239eed04a990f00e7403e4c8065927de010fd17a"
    );
}
