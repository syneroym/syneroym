//! Publishing a hosted service's endpoint record under its member master DID.
//!
//! A substrate holds a delegated instance key for each member it hosts, never
//! the member's master key (ADR-0020 §3). This builds the record that key is
//! entitled to publish: keyed by the master DID, signed by the instance key,
//! carrying the certificate that binds them.

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use syneroym_identity::{Identity, substrate};

use crate::{
    dht_registry::{EndpointInfo, EndpointType, RecordTrust, RegistryClient, SignedEndpointInfo},
    local_registry::EndpointRegistry,
};

#[derive(Debug)]
pub struct EndpointPublisher {
    registry_client: Arc<RegistryClient>,
    registry: EndpointRegistry,
    node_identity: Arc<Identity>,
    node_did: String,
    hosted_apps_dir: PathBuf,
}

impl EndpointPublisher {
    pub fn new(
        registry_client: Arc<RegistryClient>,
        registry: EndpointRegistry,
        node_identity: Arc<Identity>,
        node_did: String,
        hosted_apps_dir: PathBuf,
    ) -> Self {
        Self { registry_client, registry, node_identity, node_did, hosted_apps_dir }
    }

    /// Publishes `service_id`'s endpoint record. `Ok(false)` means there was
    /// nothing to publish -- no installed certificate and no stored record --
    /// which is a normal state, not a failure.
    pub async fn publish_service(&self, service_id: &str) -> anyhow::Result<bool> {
        let Some(record) = self.build_record(service_id) else {
            return Ok(false);
        };
        self.registry_client.register(&record, false).await?;
        Ok(true)
    }

    /// Every hosted service. A per-service failure is warned and the sweep
    /// continues; this is the retry path for a deploy-time publish that
    /// failed, so one unreachable record must not stop the rest.
    pub async fn publish_all_services(&self) {
        let mut ids: BTreeSet<String> =
            self.registry.all_instance_certs().into_iter().map(|(id, _)| id).collect();

        if let Ok(mut entries) = tokio::fs::read_dir(&self.hosted_apps_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                    ids.insert(stem.to_string());
                }
            }
        }

        for id in ids {
            match self.publish_service(&id).await {
                Ok(true) => tracing::debug!(service_id = %id, "published endpoint record"),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(service_id = %id, %e, "failed to publish endpoint record");
                }
            }
        }
    }

    /// Split out from `publish_service` so the whole decision table is
    /// testable without a registry to publish to.
    fn build_record(&self, service_id: &str) -> Option<SignedEndpointInfo> {
        // On the certificate path this blob is metadata only, and its
        // signature is neither trusted nor checked: with
        // `--instance-certificate` it is signed by an ephemeral key and
        // cannot self-verify (D-A1-8). Safe because a service with a
        // certificate never republishes it verbatim -- and the no-certificate
        // branch below verifies before it does.
        let stored_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        let stored: Option<SignedEndpointInfo> =
            std::fs::read_to_string(&stored_path).ok().and_then(|s| serde_json::from_str(&s).ok());

        let Some(cert) = self.registry.instance_cert(service_id) else {
            // Pre-A0 path: replay, but only a record that still verifies. A
            // service redeployed without a certificate can leave an earlier
            // metadata envelope behind (`deploy` overwrites this file only
            // when the manifest carries one), and re-POSTing it would earn a
            // 401 the operator has no way to explain.
            let record = stored?;
            if record.verify(RecordTrust::Publishing).is_err() {
                tracing::warn!(
                    service_id = %service_id,
                    "stored endpoint record no longer verifies; not republishing"
                );
                return None;
            }
            return Some(record);
        };

        if cert.is_expired() {
            tracing::warn!(service_id = %service_id, "instance certificate expired; not republishing");
            return None;
        }

        let Some(owner) = self.registry.owner_of(service_id) else {
            tracing::warn!(service_id = %service_id, "no recorded owner; cannot derive the instance key");
            return None;
        };

        let instance = self.node_identity.derive_service_identity(&owner, service_id);
        let instance_did = substrate::derive_did_key(&instance.public_key());
        if instance_did != cert.temporary_did {
            // owner row and certificate have drifted apart -- a redeploy by a
            // different operator, most likely. Publishing under a key the
            // certificate does not name would produce a record nothing
            // accepts.
            tracing::warn!(
                service_id = %service_id,
                "derived instance key does not match the installed certificate"
            );
            return None;
        }

        let info = EndpointInfo {
            service_id: service_id.to_string(), // overwritten by sign_as_instance
            substrate_id: self.node_did.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: stored.as_ref().and_then(|s| s.info.nickname.clone()),
            is_private: stored.as_ref().is_some_and(|s| s.info.is_private),
            ttl: None, // never from the blob -- D-A1-10
            delegation: None,
        };

        match info.sign_as_instance(&instance, cert) {
            Ok(record) => Some(record),
            Err(e) => {
                tracing::warn!(service_id = %service_id, %e, "failed to sign endpoint record as instance");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use syneroym_identity::{DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE};
    use tempfile::TempDir;

    use super::*;
    use crate::storage::MockStorage;

    async fn new_registry() -> EndpointRegistry {
        EndpointRegistry::new(Arc::new(MockStorage::default())).await.unwrap()
    }

    fn publisher(
        registry: EndpointRegistry,
        node_identity: Identity,
        hosted_apps_dir: PathBuf,
    ) -> EndpointPublisher {
        let node_did = substrate::derive_did_key(&node_identity.public_key());
        EndpointPublisher::new(
            Arc::new(RegistryClient::new(false, None)),
            registry,
            Arc::new(node_identity),
            node_did,
            hosted_apps_dir,
        )
    }

    fn write_stored_record(dir: &std::path::Path, service_id: &str, signed: &SignedEndpointInfo) {
        std::fs::write(
            dir.join(format!("{service_id}.json")),
            serde_json::to_string(signed).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn a_service_with_an_installed_certificate_publishes_under_its_master_did() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let node_did = substrate::derive_did_key(&node_identity.public_key());
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert.clone()).await.unwrap();

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        let record = pub_.build_record("svc-1").expect("a record should be built");

        assert_eq!(record.info.service_id, master_did);
        assert_eq!(record.info.substrate_id, node_did);
        assert_eq!(record.info.delegation.as_ref().unwrap().temporary_did, cert.temporary_did);
        assert!(record.verify(RecordTrust::Publishing).is_ok());
    }

    #[tokio::test]
    async fn an_expired_certificate_publishes_nothing() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            0,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").is_none());
    }

    #[tokio::test]
    async fn a_service_without_a_certificate_replays_its_stored_record_verbatim() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            delegation: None,
        };
        let signed = info.sign(&identity).unwrap();
        write_stored_record(tmp.path(), &did, &signed);

        let registry = new_registry().await;
        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        let record = pub_.build_record(&did).expect("the stored record should be replayed");

        assert_eq!(record.info.service_id, signed.info.service_id);
        assert_eq!(record.pkarr_packet_hex, signed.pkarr_packet_hex);
    }

    #[tokio::test]
    async fn the_published_record_keeps_the_stored_records_nickname_and_privacy() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        // A metadata-only envelope, signed by an unrelated ephemeral key
        // (D-A1-8's `--instance-certificate` shape): it does not self-verify,
        // and does not need to.
        let ephemeral = Identity::generate().unwrap();
        let stored_info = EndpointInfo {
            service_id: substrate::derive_did_key(&ephemeral.public_key()),
            substrate_id: "irrelevant".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("member-one".to_string()),
            is_private: true,
            ttl: Some(999_999),
            delegation: None,
        };
        let stored_signed = stored_info.sign(&ephemeral).unwrap();
        write_stored_record(tmp.path(), "svc-1", &stored_signed);

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        let record = pub_.build_record("svc-1").expect("a record should be built");

        assert_eq!(record.info.nickname, Some("member-one".to_string()));
        assert!(record.info.is_private);
    }

    #[tokio::test]
    async fn a_service_with_no_recorded_owner_publishes_nothing() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        // No `set_owner` call -- the certificate names an instance key
        // derived from an owner nothing recorded.
        let other_identity = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            other_identity.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").is_none());
    }

    #[tokio::test]
    async fn a_certificate_naming_a_key_this_node_does_not_derive_publishes_nothing() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();

        // The certificate names a key this node's `derive_service_identity`
        // for (owner, "svc-1") will never produce -- e.g. issued for a
        // different node's derived instance key, or by hand.
        let unrelated = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            unrelated.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").is_none());
    }

    #[tokio::test]
    async fn a_stored_record_that_does_not_self_verify_still_supplies_its_metadata() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        // D-A1-8's third shape: signed by a throwaway key unrelated to
        // service_id, so this record can never self-verify.
        let ephemeral = Identity::generate().unwrap();
        let stored_info = EndpointInfo {
            service_id: "svc-1".to_string(),
            substrate_id: "irrelevant".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("member-one".to_string()),
            is_private: false,
            ttl: None,
            delegation: None,
        };
        let stored_signed = stored_info.sign(&ephemeral).unwrap();
        write_stored_record(tmp.path(), "svc-1", &stored_signed);

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        let record = pub_.build_record("svc-1").expect("a record should still be built");

        assert_eq!(record.info.nickname, Some("member-one".to_string()));
    }

    #[tokio::test]
    async fn a_stored_record_that_no_longer_verifies_is_not_replayed() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();

        // The same envelope as the previous test, but with no certificate
        // installed -- so this service is on the `None`-certificate replay
        // path, which must not replay an unverifiable file.
        let ephemeral = Identity::generate().unwrap();
        let stored_info = EndpointInfo {
            service_id: "svc-1".to_string(),
            substrate_id: "irrelevant".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("member-one".to_string()),
            is_private: false,
            ttl: None,
            delegation: None,
        };
        let stored_signed = stored_info.sign(&ephemeral).unwrap();
        write_stored_record(tmp.path(), "svc-1", &stored_signed);

        let registry = new_registry().await;
        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").is_none());
    }

    #[tokio::test]
    async fn the_published_record_ignores_a_stored_ttl() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();

        let registry = new_registry().await;
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        let ephemeral = Identity::generate().unwrap();
        let stored_info = EndpointInfo {
            service_id: "svc-1".to_string(),
            substrate_id: "irrelevant".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: Some(999_999_999),
            delegation: None,
        };
        let stored_signed = stored_info.sign(&ephemeral).unwrap();
        write_stored_record(tmp.path(), "svc-1", &stored_signed);

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());
        let record = pub_.build_record("svc-1").expect("a record should be built");

        assert_eq!(record.info.ttl, None);
    }

    #[tokio::test]
    async fn the_sweep_covers_certified_and_stored_only_services_and_survives_one_failing() {
        let tmp = TempDir::new().unwrap();
        let node_identity = Identity::generate().unwrap();
        let master = Identity::generate().unwrap();
        let registry = new_registry().await;

        // Certified service 1: publishable.
        let owner = "did:key:zOwner".to_string();
        registry.set_owner("svc-1".to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, "svc-1");
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();

        // Certified service 2: expired, so it fails to publish -- the sweep
        // must not stop because of it.
        let other_instance = Identity::generate().unwrap();
        let expired_cert = DelegationCertificate::issue(
            &master,
            other_instance.public_key(),
            0,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert("svc-2".to_string(), expired_cert).await.unwrap();

        // Stored-only service 3: no certificate, replays its file.
        let identity3 = Identity::generate().unwrap();
        let did3 = substrate::derive_did_key(&identity3.public_key());
        let info3 = EndpointInfo {
            service_id: did3.clone(),
            substrate_id: did3.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            delegation: None,
        };
        write_stored_record(tmp.path(), &did3, &info3.sign(&identity3).unwrap());

        let pub_ = publisher(registry, node_identity, tmp.path().to_path_buf());

        assert!(pub_.build_record("svc-1").is_some());
        assert!(pub_.build_record("svc-2").is_none());
        assert!(pub_.build_record(&did3).is_some());
    }
}
