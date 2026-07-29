//! Publishing a hosted service's endpoint record under its member master DID.
//!
//! A substrate holds only a delegated instance key for each member it hosts,
//! never the member's master key (ADR-0020 §3) -- so it cannot sign an
//! endpoint record under the master DID itself. The deployer signs it once,
//! at deploy time, and hands the substrate the finished, self-verifying
//! blob; the substrate's whole job here is to store that blob and replay it
//! verbatim on every heartbeat until it stops verifying (expiry, or a
//! superseding record deployed elsewhere).

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use crate::dht_registry::{RegistryClient, SignedEndpointInfo};

#[derive(Debug)]
pub struct EndpointPublisher {
    registry_client: Arc<RegistryClient>,
    hosted_apps_dir: PathBuf,
}

impl EndpointPublisher {
    pub fn new(registry_client: Arc<RegistryClient>, hosted_apps_dir: PathBuf) -> Self {
        Self { registry_client, hosted_apps_dir }
    }

    /// The registry client this publisher publishes through. Lets a caller
    /// that already holds an `EndpointPublisher` reuse its client instead of
    /// building a second one from the same config -- each opens its own
    /// pkarr DHT client when the DHT is enabled.
    #[must_use]
    pub fn registry_client(&self) -> Arc<RegistryClient> {
        self.registry_client.clone()
    }

    /// Publishes `service_id`'s endpoint record. `Ok(false)` means there was
    /// no stored record to publish, which is a normal state (a service
    /// deployed without `--identity`/`--master`), not a failure.
    pub async fn publish_service(&self, service_id: &str) -> anyhow::Result<bool> {
        let Some(record) = self.build_record(service_id).await else {
            return Ok(false);
        };
        self.registry_client.register(&record, false).await?;
        Ok(true)
    }

    /// Every hosted service with a stored record. A per-service failure is
    /// warned and the sweep continues; this is the retry path for a
    /// deploy-time publish that failed, so one unreachable record must not
    /// stop the rest.
    pub async fn publish_all_services(&self) {
        let mut ids: BTreeSet<String> = BTreeSet::new();

        if let Ok(mut entries) = tokio::fs::read_dir(&self.hosted_apps_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(file_type) = entry.file_type().await
                    && file_type.is_file()
                    && let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str())
                {
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
    /// testable without a registry to publish to. There is nothing left to
    /// build here -- the substrate has no key this record could ever be
    /// signed with (a member's is the deployer's master key; ADR-0020 §3),
    /// so this only ever replays the deployer-signed blob it was handed at
    /// deploy, and only while that blob still verifies.
    async fn build_record(&self, service_id: &str) -> Option<SignedEndpointInfo> {
        let stored_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        let record: SignedEndpointInfo = tokio::fs::read_to_string(&stored_path)
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())?;

        if let Err(e) = record.verify() {
            tracing::warn!(
                service_id = %service_id,
                %e,
                "stored endpoint record no longer verifies; not republishing"
            );
            return None;
        }

        Some(record)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use syneroym_identity::{Identity, substrate};
    use tempfile::TempDir;

    use super::*;
    use crate::dht_registry::{EndpointInfo, EndpointType};

    fn publisher(hosted_apps_dir: PathBuf) -> EndpointPublisher {
        EndpointPublisher::new(Arc::new(RegistryClient::new(false, None)), hosted_apps_dir)
    }

    fn write_stored_record(dir: &Path, service_id: &str, signed: &SignedEndpointInfo) {
        fs::write(dir.join(format!("{service_id}.json")), serde_json::to_string(signed).unwrap())
            .unwrap();
    }

    fn sample_info(service_id: &str, not_after: u64) -> EndpointInfo {
        EndpointInfo {
            service_id: service_id.to_string(),
            substrate_id: "did:key:zSubstrate".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after,
        }
    }

    fn far_future() -> u64 {
        u64::MAX / 2
    }

    #[tokio::test]
    async fn a_master_signed_record_is_replayed_verbatim() {
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let signed = sample_info(&master_did, far_future()).sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        let record =
            pub_.build_record(&master_did).await.expect("the stored record should be replayed");

        assert_eq!(record.info.service_id, master_did);
        assert_eq!(record.pkarr_packet_hex, signed.pkarr_packet_hex);
    }

    #[tokio::test]
    async fn a_record_signed_by_a_key_other_than_its_own_service_id_is_not_replayed() {
        let tmp = TempDir::new().unwrap();
        // Signed by an unrelated key, but claiming a different service_id --
        // the substrate has no way to produce this signature itself, so a
        // record like this could only ever be a bad file on disk, never
        // something the deploy path itself writes.
        let ephemeral = Identity::generate().unwrap();
        let stored_signed =
            sample_info("did:key:zSomeoneElse", far_future()).sign(&ephemeral).unwrap();
        write_stored_record(tmp.path(), "svc-1", &stored_signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").await.is_none());
    }

    #[tokio::test]
    async fn an_expired_stored_record_is_not_replayed() {
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let signed = sample_info(&master_did, 1).sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.build_record(&master_did).await.is_none());
    }

    #[tokio::test]
    async fn no_stored_record_publishes_nothing() {
        let tmp = TempDir::new().unwrap();
        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").await.is_none());
    }

    #[tokio::test]
    async fn the_published_record_keeps_the_stored_records_nickname_and_privacy() {
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let mut info = sample_info(&master_did, far_future());
        info.nickname = Some("member-one".to_string());
        info.is_private = true;
        let signed = info.sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        let record = pub_.build_record(&master_did).await.expect("a record should be built");

        assert_eq!(record.info.nickname, Some("member-one".to_string()));
        assert!(record.info.is_private);
    }
}
