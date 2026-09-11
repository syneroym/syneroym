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
    use syneroym_identity::Identity;

    use super::*;

    fn sample_record() -> SignedEndpointInfo {
        let identity = Identity::generate().unwrap();
        let service_id = syneroym_identity::substrate::derive_did_key(&identity.public_key());
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
        }
        .sign(&identity)
        .unwrap()
    }

    #[test]
    fn private_splits_to_no_certificate() {
        let (visibility, cert) = Publication::Private.split().unwrap();
        assert_eq!(visibility, Visibility::Private);
        assert!(cert.is_none());
    }

    #[test]
    fn public_and_internal_split_to_a_serialized_certificate() {
        let (visibility, cert) = Publication::Public(sample_record()).split().unwrap();
        assert_eq!(visibility, Visibility::Public);
        assert!(cert.is_some());

        let (visibility, cert) = Publication::Internal(sample_record()).split().unwrap();
        assert_eq!(visibility, Visibility::Internal);
        assert!(cert.is_some());
    }
}

#[cfg(test)]
mod new_with_record_tests {
    use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
    use syneroym_identity::Identity;

    use super::*;

    #[tokio::test]
    async fn new_with_record_verifies_signature_and_sets_fields() {
        // `verify()` resolves the signing key from `service_id` itself
        // (the registry's own admission rule), so it must be the signer's
        // real derived DID, not a placeholder string.
        let identity = Identity::generate().unwrap();
        let service_id = syneroym_identity::substrate::derive_did_key(&identity.public_key());
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
