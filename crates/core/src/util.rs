//! Common utilities and helpers.

use std::{env, fs, path::Path};

use syneroym_identity::Identity;

use crate::{
    config::{DEFAULT_SUBSTRATE_KEY_FILE, SubstrateConfig},
    protocol_utils::SHORT_HASH_LEN,
};

/// Loads the node's own substrate identity, the same key file
/// `syneroym_substrate::identity::setup_substrate_identity` loads (by the
/// same path-resolution rule) -- generating and persisting it if this is
/// the first component to run. Shared by the client gateway and the
/// WebRTC coordinator (S3, D-S3-6): both present this same node identity
/// to satisfy `[iam].grant_resolve_to_node_did`, and using the node's own
/// key rather than a fresh one per component is what lets one config key
/// cover both.
pub fn load_or_generate_node_identity(config: &SubstrateConfig) -> anyhow::Result<Identity> {
    let key_path = config
        .identity
        .key
        .clone()
        .unwrap_or_else(|| config.app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if key_path.exists() {
        Identity::load_from_path(&key_path)
    } else {
        let id = Identity::generate()?;
        id.save_to_path(&key_path)?;
        Ok(id)
    }
}

/// Parses a string representing a size into a number of bytes.
///
/// Supports common suffixes like `Ki`, `Mi`, `Gi`, `K`, `M`, `G`.
/// If the string cannot be parsed as a number, it returns
/// `default_if_unparseable` multiplied by the parsed suffix multiplier.
#[must_use]
pub fn parse_size_string(s: &str, default_if_unparseable: u64) -> u64 {
    let s = s.trim();
    let mut multiplier = 1;
    let num_str = if let Some(stripped) = s.strip_suffix("Gi") {
        multiplier = 1024 * 1024 * 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("Mi") {
        multiplier = 1024 * 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("Ki") {
        multiplier = 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("G") {
        multiplier = 1000 * 1000 * 1000;
        stripped
    } else if let Some(stripped) = s.strip_suffix("M") {
        multiplier = 1000 * 1000;
        stripped
    } else if let Some(stripped) = s.strip_suffix("K") {
        multiplier = 1000;
        stripped
    } else {
        s
    };

    num_str.trim().parse::<u64>().unwrap_or(default_if_unparseable) * multiplier
}

/// Generates a short z32-encoded hash from the given data.
/// It uses SHA256 and takes the first 5 bytes, resulting in an 8-character
/// string.
#[must_use]
pub fn short_hash(data: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    z32::encode(&result[..5])
}

/// The registry's alias for a nicknamed DID: `<nickname>-<short_hash(did)>`,
/// or a bare `<short_hash(did)>` when there is no nickname.
///
/// Unambiguous even when the nickname contains dashes, because
/// `short_hash` is always exactly 8 characters and always last. Carries no
/// role letter deliberately: a gateway host reconstructs this same string
/// from either its `-a` segment or its `-s` segment, and a letter here
/// would have to stand for both (D-S3-14).
#[must_use]
pub fn generate_alias(nickname: Option<&str>, service_id: &str) -> String {
    let service_hash = short_hash(service_id);
    match nickname {
        Some(n) => format!("{n}-{service_hash}"),
        None => service_hash,
    }
}

/// The longest a single DNS label may be. A host label over this is
/// rejected by resolvers and browsers before anything of ours sees it.
pub const MAX_DNS_LABEL_LEN: usize = 63;

fn refuse_ambiguous_nickname_tail(nickname: &str) -> anyhow::Result<()> {
    // §0.12's irreducible residue: a nickname whose own final dash-segment
    // is `a` followed by exactly 8 characters is genuinely ambiguous to
    // any parser (it is indistinguishable from a real `-a<hash>`
    // segment), so it is refused where it is minted rather than misread
    // where it is used. Only `-a` needs this guard: `-s` is required and
    // `-i` is popped before it, so neither can ever inspect a nickname
    // segment.
    if let Some(last) = nickname.split('-').next_back()
        && last.len() == 9
        && last.starts_with('a')
    {
        anyhow::bail!(
            "nickname '{nickname}' ends in a segment ('{last}') that looks like an `-a<hash>` app \
             segment; rename it so the gateway host it builds can be parsed unambiguously"
        );
    }
    Ok(())
}

/// `hashed_suffix_len` is the length of everything in `label` after the
/// nickname (the hashed segments), so a refusal can name the nickname
/// budget that was actually available, not just the overall limit.
fn finish_host(label: String, domain: &str, hashed_suffix_len: usize) -> anyhow::Result<String> {
    anyhow::ensure!(
        label.len() <= MAX_DNS_LABEL_LEN,
        "gateway host label '{label}' is {} characters, over the {MAX_DNS_LABEL_LEN}-character \
         DNS label limit ({} characters are left for the nickname)",
        label.len(),
        MAX_DNS_LABEL_LEN.saturating_sub(hashed_suffix_len)
    );
    Ok(format!("{label}.{domain}"))
}

/// `<nickname>-s<service-did-hash>[-i<interface-hash>].<domain>` -- a
/// service that belongs to no app instance.
///
/// # Errors
/// The label exceeds [`MAX_DNS_LABEL_LEN`], or `nickname`'s own final
/// segment would misread as an `-a<hash>` segment (§0.12) -- the parser
/// pops an optional `-a` last, so an unscoped host is exposed to the same
/// ambiguity an app-scoped one is.
/// `interface: None` omits the `-i` segment: the destination resolves it
/// to the service's one app-declared interface (D-S3-15).
pub fn generate_service_host(
    nickname: Option<&str>,
    service_id: &str,
    interface: Option<&str>,
    domain: &str,
) -> anyhow::Result<String> {
    let nickname = nickname.unwrap_or_default();
    refuse_ambiguous_nickname_tail(nickname)?;
    let s_hash = short_hash(service_id);
    let mut label = if nickname.is_empty() { String::new() } else { format!("{nickname}-") };
    label.push_str(&format!("s{s_hash}"));
    let mut hashed_suffix_len = 1 + SHORT_HASH_LEN;
    if let Some(iface) = interface {
        label.push_str(&format!("-i{}", short_hash(iface)));
        hashed_suffix_len += 2 + SHORT_HASH_LEN;
    }
    finish_host(label, domain, hashed_suffix_len)
}

/// `<nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>]
/// .<domain>` -- a logical service of an app instance (ADR-0022 §7).
///
/// `nickname` must be the app instance's `AppInstanceId`:
/// `<nickname>-<app did hash>` is the registry alias the app's Tier-1
/// record was admitted under, and reconstructing it is what lets a reader
/// recover the app DID with no new registry record type.
///
/// # Errors
/// The label exceeds [`MAX_DNS_LABEL_LEN`] (the three hashed segments cost
/// 30, so the nickname budget is 33 -- 43 with no `-i`), or `nickname`'s
/// own final segment would misread as an `-a<hash>` segment (§0.12).
/// `interface: None` omits the `-i` segment (D-S3-15).
pub fn generate_app_host(
    nickname: &str,
    app_did: &str,
    service_name: &str,
    interface: Option<&str>,
    domain: &str,
) -> anyhow::Result<String> {
    refuse_ambiguous_nickname_tail(nickname)?;
    let a_hash = short_hash(app_did);
    let s_hash = short_hash(service_name);
    let mut label = format!("{nickname}-a{a_hash}-s{s_hash}");
    let mut hashed_suffix_len = 2 * (1 + SHORT_HASH_LEN);
    if let Some(iface) = interface {
        label.push_str(&format!("-i{}", short_hash(iface)));
        hashed_suffix_len += 2 + SHORT_HASH_LEN;
    }
    finish_host(label, domain, hashed_suffix_len)
}

pub fn read_local_artifact(path: &Path) -> anyhow::Result<Vec<u8>> {
    fs::read(path)
        .or_else(|_| {
            if let Ok(cwd) = env::current_dir() {
                let target = cwd.join(path);
                if target.exists() {
                    return fs::read(&target);
                }
            }
            fs::read(path)
        })
        .map_err(|e| anyhow::anyhow!("Failed to read file at {:?}: {}", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_string() {
        assert_eq!(parse_size_string("1024", 128), 1024);
        assert_eq!(parse_size_string("1K", 128), 1000);
        assert_eq!(parse_size_string("1Ki", 128), 1024);
        assert_eq!(parse_size_string("2 M", 128), 2000000);
        assert_eq!(parse_size_string("500Mi", 128), 500 * 1024 * 1024);
        assert_eq!(parse_size_string("invalidGi", 128), 128 * 1024 * 1024 * 1024);
    }

    /// Test 68: D-S3-2's whole load-bearing claim -- the alias a gateway
    /// host's `-a` segment reconstructs must equal the alias the registry
    /// actually admitted the Tier-1 record under (`register_endpoint`'s
    /// `generate_alias(nickname, service_id)`, where `nickname` is the app
    /// instance id and `service_id` is the app DID).
    #[test]
    fn the_reconstructed_app_alias_matches_what_the_registry_admitted() {
        let app_instance_id = "my-chat-app";
        let app_did = "did:key:zAppMaster";
        let service_name = "backend";

        let admitted_alias = generate_alias(Some(app_instance_id), app_did);

        let host =
            generate_app_host(app_instance_id, app_did, service_name, None, "localhost").unwrap();
        let parsed = crate::protocol_utils::parse_target_host(&host).unwrap();
        let crate::protocol_utils::TargetHost::App { app_lookup_alias, .. } = parsed else {
            panic!("expected an App target, got {parsed:?}");
        };

        assert_eq!(app_lookup_alias, admitted_alias);
    }

    /// Test 69: D-S3-9, on both builders, asserting the message names 63.
    #[test]
    fn a_host_label_over_the_dns_limit_is_refused_not_truncated() {
        let long_nickname = "n".repeat(60);
        let err = generate_service_host(Some(&long_nickname), "did:key:zSvc", None, "localhost")
            .unwrap_err();
        assert!(err.to_string().contains("63"), "{err}");

        let err = generate_app_host(&long_nickname, "did:key:zApp", "backend", None, "localhost")
            .unwrap_err();
        assert!(err.to_string().contains("63"), "{err}");
    }

    /// Test 70: §0.12's irreducible residue -- refused at build, not
    /// misread at parse. Both builders share the guard: an unscoped host
    /// is popped through the same optional-`-a` step an app-scoped one is,
    /// so it is exposed to the identical ambiguity (finding A1).
    #[test]
    fn a_nickname_whose_last_segment_looks_like_an_app_hash_is_refused_at_build() {
        let fake_hash = "a".to_string() + &"b".repeat(SHORT_HASH_LEN);
        let nickname = format!("data-{fake_hash}");

        let err =
            generate_app_host(&nickname, "did:key:zApp", "backend", None, "localhost").unwrap_err();
        assert!(err.to_string().contains(&fake_hash), "{err}");

        let err =
            generate_service_host(Some(&nickname), "did:key:zSvc", None, "localhost").unwrap_err();
        assert!(err.to_string().contains(&fake_hash), "{err}");
    }

    /// Test 71: both forms, property-style over a handful of
    /// nickname/name/interface shapes, including a dashed nickname and an
    /// absent one.
    #[test]
    fn a_built_host_round_trips_through_the_parser() {
        use crate::protocol_utils::{TargetHost, parse_target_host};

        for (nickname, interface) in
            [(Some("svc"), Some("default")), (Some("my-dashed-svc"), None), (None, None)]
        {
            let host =
                generate_service_host(nickname, "did:key:zSvc", interface, "localhost").unwrap();
            let parsed = parse_target_host(&host).unwrap();
            let expected_alias = generate_alias(nickname, "did:key:zSvc");
            assert_eq!(
                parsed,
                TargetHost::Service {
                    lookup_alias: expected_alias,
                    interface: interface.map(short_hash).unwrap_or_default(),
                }
            );
        }

        for (nickname, interface) in [("app", Some("default")), ("my-dashed-app", None)] {
            let host =
                generate_app_host(nickname, "did:key:zApp", "backend", interface, "localhost")
                    .unwrap();
            let parsed = parse_target_host(&host).unwrap();
            assert_eq!(
                parsed,
                TargetHost::App {
                    app_lookup_alias: generate_alias(Some(nickname), "did:key:zApp"),
                    app_did_hash: short_hash("did:key:zApp"),
                    service_name_hash: short_hash("backend"),
                    interface: interface.map(short_hash).unwrap_or_default(),
                }
            );
        }
    }

    /// Test 72: D-S3-14's uniqueness claim now that the alias carries no
    /// letter -- `short_hash` is always 8 characters and always last, so
    /// `rsplit_once('-')` recovers `(nickname, hash)` even when the
    /// nickname itself contains dashes.
    #[test]
    fn an_alias_with_a_dashed_nickname_still_splits_at_the_hash() {
        let alias = generate_alias(Some("a-b-c"), "did:key:zSvc");
        let (nickname, hash) = alias.rsplit_once('-').unwrap();
        assert_eq!(nickname, "a-b-c");
        assert_eq!(hash, short_hash("did:key:zSvc"));

        let bare_alias = generate_alias(Some("a"), "did:key:zSvc");
        let (nickname, hash) = bare_alias.rsplit_once('-').unwrap();
        assert_eq!(nickname, "a");
        assert_eq!(hash, short_hash("did:key:zSvc"));
    }
}
