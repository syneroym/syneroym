//! Publishing a hosted service's endpoint record under its member master DID.
//!
//! A substrate holds only a delegated instance key for each member it hosts,
//! never the member's master key (ADR-0020 §3) -- so it cannot sign an
//! endpoint record under the master DID itself. The deployer signs it once,
//! at deploy time, and hands the substrate the finished, self-verifying
//! blob; the substrate's whole job here is to store that blob and replay it
//! verbatim on every heartbeat until it stops verifying (expiry, or a
//! superseding record deployed elsewhere).

use std::{
    collections::BTreeSet,
    io,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::dht_registry::{RegistryClient, SignedEndpointInfo};

/// How long before a stored record's own `not_after` this warns, on the same
/// hourly sweep `runtime.rs`'s `warn_on_near_expiry_instance_certs` uses for
/// the instance certificate. Unlike a certificate, a record carries no
/// `issued_at` to compute a lifetime fraction from -- only `not_after` itself
/// -- so this is a fixed window rather than a percentage. A week is
/// comfortably inside `DEFAULT_ENDPOINT_NOT_AFTER_SECS`'s 30-day default,
/// leaving room to act before the record silently drops out of resolution
/// (`crates/core/src/dht_registry.rs`'s `DEFAULT_ENDPOINT_NOT_AFTER_SECS`).
const NOT_AFTER_WARNING_WINDOW_SECS: u64 = 7 * 24 * 3600;

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
        for id in self.hosted_ids().await {
            match self.publish_service(&id).await {
                Ok(true) => tracing::debug!(service_id = %id, "published endpoint record"),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(service_id = %id, %e, "failed to publish endpoint record");
                }
            }
        }
    }

    /// Warns for any stored, still-verifying record within
    /// `NOT_AFTER_WARNING_WINDOW_SECS` of its own `not_after`, and returns
    /// their ids. Split out from any sleep loop -- mirroring
    /// `runtime.rs`'s `warn_on_near_expiry_instance_certs` -- so it is
    /// testable without waiting on a real timer.
    ///
    /// A record that no longer verifies at all (already expired, or
    /// otherwise invalid) is not this method's concern: `build_record`
    /// already warns for that case on every publish attempt. This warns
    /// *before* that happens, while there is still time for the deployer to
    /// re-sign and redeploy -- there is no `certify-instance`-style renewal
    /// verb for a record alone yet (deferred backlog).
    pub async fn warn_on_near_expiry_records(&self) -> Vec<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let mut near_expiry = Vec::new();
        for id in self.hosted_ids().await {
            let Some(record) = self.build_record(&id).await else { continue };
            let remaining = record.info.not_after.saturating_sub(now);
            if remaining <= NOT_AFTER_WARNING_WINDOW_SECS {
                tracing::warn!(
                    service_id = %id,
                    not_after = record.info.not_after,
                    remaining_secs = remaining,
                    "endpoint record is within its warning window of not_after -- re-sign and \
                     redeploy before it lapses, which drops the service out of name \
                     resolution"
                );
                near_expiry.push(id);
            }
        }
        near_expiry
    }

    /// The set of hosted service ids with a stored record file, shared by
    /// `publish_all_services` and `warn_on_near_expiry_records` so both walk
    /// the same directory the same way.
    async fn hosted_ids(&self) -> BTreeSet<String> {
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

        ids
    }

    /// Split out from `publish_service` so the whole decision table is
    /// testable without a registry to publish to. There is nothing left to
    /// build here -- the substrate has no key this record could ever be
    /// signed with (a member's is the deployer's master key; ADR-0020 §3),
    /// so this only ever replays the deployer-signed blob it was handed at
    /// deploy, and only while that blob still verifies.
    async fn build_record(&self, service_id: &str) -> Option<SignedEndpointInfo> {
        let stored_path = self.hosted_apps_dir.join(format!("{service_id}.json"));

        // A missing file is the normal, silent case: a service deployed
        // without `--identity`/`--master` has no record and never will.
        // Anything else that keeps this from becoming a `SignedEndpointInfo`
        // -- an I/O error on a file that does exist, or one that exists but
        // does not parse -- is not that normal case and must not look like
        // it: `publish_all_services` walks the directory itself, so every
        // id it hands here already has a file, and a silent skip there is a
        // silently broken record, not an absent one.
        let contents = match tokio::fs::read_to_string(&stored_path).await {
            Ok(contents) => contents,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!(
                    service_id = %service_id,
                    %e,
                    "failed to read stored endpoint record; not republishing"
                );
                return None;
            }
        };

        let record: SignedEndpointInfo = match serde_json::from_str(&contents) {
            Ok(record) => record,
            Err(e) => {
                tracing::warn!(
                    service_id = %service_id,
                    %e,
                    "stored endpoint record does not parse; not republishing"
                );
                return None;
            }
        };

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
            generation: 0,
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
    async fn a_stored_file_that_does_not_parse_is_not_replayed() {
        // Not a synthetic case: any file written before `not_after` became
        // a required field (no `serde` default) fails exactly this way,
        // pre-release with no migration -- this proves that failure is
        // silent-but-safe (`None`, no panic), not silently indistinguishable
        // from a missing file in a way that would matter to a caller.
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("svc-1.json"), r#"{"not":"a record"}"#).unwrap();

        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.build_record("svc-1").await.is_none());
    }

    fn now_secs() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    #[tokio::test]
    async fn warn_on_near_expiry_records_flags_a_record_inside_the_window() {
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        // One hour from now -- well inside the 7-day warning window, but
        // still live, so `build_record` must still return it.
        let signed = sample_info(&master_did, now_secs() + 3600).sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        assert_eq!(pub_.warn_on_near_expiry_records().await, vec![master_did]);
    }

    #[tokio::test]
    async fn warn_on_near_expiry_records_ignores_a_record_far_in_the_future() {
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let signed = sample_info(&master_did, far_future()).sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.warn_on_near_expiry_records().await.is_empty());
    }

    #[tokio::test]
    async fn warn_on_near_expiry_records_ignores_an_already_expired_record() {
        // Not this method's job -- `build_record` already warns for a
        // record that no longer verifies at all, on every publish attempt.
        let tmp = TempDir::new().unwrap();
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let signed = sample_info(&master_did, 1).sign(&master).unwrap();
        write_stored_record(tmp.path(), &master_did, &signed);

        let pub_ = publisher(tmp.path().to_path_buf());
        assert!(pub_.warn_on_near_expiry_records().await.is_empty());
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
