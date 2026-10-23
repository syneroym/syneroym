#![allow(clippy::cognitive_complexity)]

use syneroym_core::dht_registry::SignedEndpointInfo;
use syneroym_rpc::framing;

use super::*;
pub(crate) use crate::client::check_frame_size;

#[cfg(test)]
mod frame_size_tests {
    use super::*;

    #[test]
    fn a_request_within_the_frame_limit_is_accepted() {
        check_frame_size("orchestrator.deploy", &vec![0u8; 1024]).unwrap();
    }

    #[test]
    fn a_request_right_at_the_limit_is_accepted() {
        check_frame_size("orchestrator.deploy", &vec![0u8; framing::MAX_FRAME_SIZE as usize])
            .unwrap();
    }

    #[test]
    fn a_request_one_byte_over_the_limit_is_refused_naming_the_method_and_both_sizes() {
        let err = check_frame_size(
            "orchestrator.deploy",
            &vec![0u8; framing::MAX_FRAME_SIZE as usize + 1],
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("orchestrator.deploy"), "{msg}");
        assert!(msg.contains(&(framing::MAX_FRAME_SIZE as usize + 1).to_string()), "{msg}");
        assert!(msg.contains(&framing::MAX_FRAME_SIZE.to_string()), "{msg}");
    }
}

#[cfg(test)]
mod publication_split_tests {
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
    use syneroym_identity::{Identity, substrate};
    use syneroym_mailbox_format::MailboxSecretKey;

    use super::*;

    fn sample_record() -> SignedEndpointInfo {
        let identity = Identity::generate().unwrap();
        let service_id = substrate::derive_did_key(&identity.public_key());
        EndpointInfo {
            service_id,
            substrate_id: "did:key:z6Mksub".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: 9999999999,
            generation: 0,
            mailbox: None,
        }
        .sign(&identity)
        .unwrap()
    }

    #[test]
    fn private_splits_to_no_certificate() {
        let parts = Publication::Private.split().unwrap();
        assert_eq!(parts.visibility, Visibility::Private);
        assert!(parts.registry_certificate.is_none());
        assert!(parts.mailbox.is_none());
    }

    #[test]
    fn public_and_internal_split_to_a_serialized_certificate() {
        let public = Publication::Public {
            record: sample_record(),
            mailbox_key: None,
            mailbox_methods: Vec::new(),
        };
        let parts = public.split().unwrap();
        assert_eq!(parts.visibility, Visibility::Public);
        assert!(parts.registry_certificate.is_some());
        assert!(parts.mailbox.is_none());

        let parts = Publication::Internal(sample_record()).split().unwrap();
        assert_eq!(parts.visibility, Visibility::Internal);
        assert!(parts.registry_certificate.is_some());
        assert!(parts.mailbox.is_none());
    }

    #[test]
    fn the_debug_text_of_split_parts_does_not_show_the_mailbox_key() {
        let key = MailboxSecretKey::generate().unwrap();
        let public = Publication::Public {
            record: sample_record(),
            mailbox_key: Some(key.clone()),
            mailbox_methods: Vec::new(),
        };
        let parts = public.split().unwrap();
        assert!(parts.mailbox.is_some());
        assert!(!format!("{parts:?}").contains(key.to_z32().as_str()));
    }

    #[test]
    fn mailbox_methods_split_to_credentials_with_the_key_only() {
        let key = MailboxSecretKey::generate().unwrap();
        let methods = vec!["greeter#greet".parse().unwrap(), "api#invoke".parse().unwrap()];
        let public = Publication::Public {
            record: sample_record(),
            mailbox_key: Some(key),
            mailbox_methods: methods,
        };
        let credentials = public.split().unwrap().mailbox.unwrap();
        assert_eq!(credentials.methods, ["greeter#greet", "api#invoke"]);

        let without_key = Publication::Public {
            record: sample_record(),
            mailbox_key: None,
            mailbox_methods: vec!["greeter#greet".parse().unwrap()],
        };
        assert!(without_key.split().unwrap().mailbox.is_none());
    }

    #[test]
    fn a_public_mailbox_key_splits_to_credentials_in_its_text_form() {
        let key = MailboxSecretKey::generate().unwrap();
        let public = Publication::Public {
            record: sample_record(),
            mailbox_key: Some(key.clone()),
            mailbox_methods: Vec::new(),
        };
        let credentials = public.split().unwrap().mailbox.unwrap();
        assert_eq!(credentials.secret_key, key.to_z32().as_str());
    }
}

#[cfg(test)]
mod new_with_record_tests {
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
    use syneroym_identity::{Identity, substrate};

    use super::*;

    #[tokio::test]
    async fn new_with_record_verifies_signature_and_sets_fields() {
        // `verify()` resolves the signing key from `service_id` itself
        // (the registry's own admission rule), so it must be the signer's
        // real derived DID, not a placeholder string.
        let identity = Identity::generate().unwrap();
        let service_id = substrate::derive_did_key(&identity.public_key());
        let substrate_id = "did:key:z6Mksub".to_string();
        let record = EndpointInfo {
            service_id: service_id.clone(),
            substrate_id: substrate_id.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("my-private-svc".to_string()),
            is_private: true,
            ttl: None,
            not_after: 9999999999,
            generation: 0,
            mailbox: None,
        }
        .sign(&identity)
        .unwrap();

        let client =
            SyneroymClient::new_with_record(record.clone(), "http://127.0.0.1:9999".to_string())
                .unwrap();
        assert_eq!(client.service_id(), service_id);
        assert_eq!(client.registry_lookup_override.as_deref(), Some(substrate_id.as_str()));
        assert!(client.provided_mechanisms.is_none());

        // Tampered record fails verification
        let mut tampered = record;
        tampered.info.service_id = "did:key:z6Mktampered".to_string();
        assert!(
            SyneroymClient::new_with_record(tampered, "http://127.0.0.1:9999".to_string()).is_err()
        );
    }
}

#[cfg(test)]
mod delegation_preamble_tests {
    use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
    use syneroym_router::RoutePreamble;

    use super::*;

    #[test]
    fn client_with_delegation_writes_delegation_on_preamble() {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();

        let client = SyneroymClient::new("did:key:z6MkTarget".to_string(), String::new())
            .with_delegation(cert.clone());

        let preamble = client.build_preamble("test_iface");
        assert_eq!(preamble.delegation, Some(cert.clone()));
        assert!(preamble.ts.is_some());
        assert!(preamble.sig.is_some());

        let line = preamble.to_preamble_line();
        let parsed = RoutePreamble::parse(&line).unwrap();
        assert_eq!(parsed.delegation, Some(cert));
        assert_eq!(parsed.ts, preamble.ts);
        assert_eq!(parsed.sig, preamble.sig);
    }
}
