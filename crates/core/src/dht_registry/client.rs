use std::time;

use bytes::Bytes;
use pkarr::{Client, PublicKey, SignedPacket, dns::rdata::RData};
use reqwest::Client as ReqwestClient;
use syneroym_identity::{Identity, substrate};

use super::{
    master_anchor::{MASTER_ANCHOR_SCHEMA_V1, MasterAnchorPayload, SignedMasterAnchor},
    types::{EndpointInfo, EndpointType, PKARR_DNS_NAME, SignedEndpointInfo},
};

/// How long a registry request waits before giving up. `reqwest::Client`
/// sets no default -- an unresponsive-but-not-refusing registry (packets
/// dropped rather than an RST) would otherwise stall the caller for the OS
/// connect timeout, which on some networks is minutes, not seconds. Every
/// call site here treats a registry failure as non-fatal already (warn and
/// continue, or fall back to the DHT), so a short timeout only trades a
/// slightly less patient retry loop for never blocking the caller this long.
const HTTP_REQUEST_TIMEOUT: time::Duration = time::Duration::from_secs(10);

#[derive(Debug)]
pub struct RegistryClient {
    dht_client: Option<Client>,
    registry_url: Option<String>,
    /// Built once and reused across every request this client makes, rather
    /// than a fresh `ReqwestClient::new()` per call: cheaper (connection
    /// pooling) and the only place the timeout above needs to be set.
    http_client: ReqwestClient,
}

async fn do_publish(dht: Client, signed_packet: SignedPacket, context: &'static str) {
    if let Err(e) = dht.publish(&signed_packet, None).await {
        tracing::warn!("Failed to publish {} to DHT: {}", context, e);
    } else {
        tracing::debug!("Successfully published {} to Mainline DHT", context);
    }
}

async fn publish_dht_packet(
    dht: Client,
    signed_packet: SignedPacket,
    sync_dht: bool,
    context: &'static str,
) {
    if sync_dht {
        do_publish(dht, signed_packet, context).await;
    } else {
        tokio::spawn(async move {
            do_publish(dht, signed_packet, context).await;
        });
    }
}

/// Pulls `id`'s `EndpointInfo` out of a resolved DHT packet and re-verifies
/// it before returning it. pkarr's own `resolve` already authenticated the
/// packet's signature against the queried pubkey, so the only thing this
/// re-parse actually checks is `not_after` -- a signer that stopped
/// renewing must eventually stop resolving on the DHT too, not just on the
/// HTTP registry's `verify()` call. `None` covers both "no matching TXT
/// record" and "found one, but it no longer verifies."
pub(crate) fn extract_verified_endpoint_from_packet(
    id: &str,
    signed_packet: &SignedPacket,
) -> Option<SignedEndpointInfo> {
    let mut found_info = None;
    for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
        if let RData::TXT(txt) = &answer.rdata
            && let Ok(full_string) = String::try_from(txt.clone())
            && let Ok(parsed_info) = serde_json::from_str::<EndpointInfo>(&full_string)
            && parsed_info.service_id == id
        {
            found_info = Some(parsed_info);
            break;
        }
    }

    let info = found_info?;
    let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
    let candidate = SignedEndpointInfo { info, pkarr_packet_hex };
    candidate.verify().ok().map(|_| candidate)
}

impl RegistryClient {
    pub fn new(enable_dht: bool, registry_url: Option<String>) -> Self {
        let dht_client = if enable_dht { Client::builder().build().ok() } else { None };
        let http_client = ReqwestClient::builder()
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| ReqwestClient::new());
        Self { dht_client, registry_url, http_client }
    }

    /// The HTTP registry URL this client publishes into, if configured.
    /// Lets a substrate report its own registry namespace rather
    /// than a caller having to guess it from config.
    #[must_use]
    pub fn registry_url(&self) -> Option<&str> {
        self.registry_url.as_deref()
    }

    /// Whether this client has a mainline DHT client configured.
    #[must_use]
    pub const fn dht_enabled(&self) -> bool {
        self.dht_client.is_some()
    }

    /// Registers the endpoint to the DHT and optionally the HTTP registry.
    pub async fn register(
        &self,
        signed_info: &SignedEndpointInfo,
        sync_dht: bool,
    ) -> anyhow::Result<()> {
        let mut http_success = self.registry_url.is_none();
        let mut published = false;

        if let Some(url) = &self.registry_url {
            let client = &self.http_client;
            let register_url = format!("{url}/register");
            tracing::debug!("Registry register: {}", register_url);

            match client.post(&register_url).json(signed_info).send().await {
                Ok(response) if response.status().is_success() => {
                    http_success = true;
                    published = true;
                }
                Ok(response) => {
                    tracing::warn!("HTTP registry returned error status: {}", response.status());
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to HTTP registry: {}", e);
                }
            }
        }

        if !http_success {
            return Err(anyhow::anyhow!("Failed to register endpoint via HTTP registry"));
        }

        // Publish to DHT (fire-and-forget in background) if HTTP succeeded or
        // wasn't configured. Every record is self-signed under a key that
        // resolves from its own `service_id`, so unlike an earlier
        // design -- where a delegation-signed record could only ever land
        // under the instance key's own DID on the DHT, never the master's --
        // this always has a home there.
        // ADR-0018 §4: `is_private` means "not propagated beyond this registry".
        // The Mainline DHT is a *wider* channel than a parent registry, so the same
        // flag has to gate it -- otherwise `internal` means "global" on any node
        // with `enable_bep0044_dht` on, which is the opposite of what it declares.
        if let Some(dht) = &self.dht_client
            && !signed_info.info.is_private
        {
            tracing::debug!("Publishing to Mainline DHT (background)");
            let packet_bytes = hex::decode(&signed_info.pkarr_packet_hex)?;
            let bytes_obj = Bytes::from(packet_bytes);
            let pubkey = substrate::resolve_did_key(&signed_info.info.service_id)?;
            let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())?;
            let signed_packet = SignedPacket::from_relay_payload(&pkarr_pubkey, &bytes_obj)?;

            // When `sync_dht` is false, `publish_dht_packet` only *starts*
            // the publish (`tokio::spawn`, fire-and-forget) and returns
            // immediately -- so `published` here means "handed to the DHT
            // publisher", not "confirmed on the DHT". `register`'s `Ok(())`
            // is honest about queuing, not about completion; a caller that
            // needs the latter must pass `sync_dht: true`.
            publish_dht_packet(dht.clone(), signed_packet, sync_dht, "endpoint info").await;
            published = true;
        }

        if !published {
            return Err(if signed_info.info.is_private {
                anyhow::anyhow!(
                    "nothing published for '{}': this node has no HTTP registry configured, and a \
                     record marked private is deliberately not published to the DHT. Configure \
                     `substrate.registry_url`, or declare this service `public`",
                    signed_info.info.service_id
                )
            } else {
                anyhow::anyhow!(
                    "nothing published for '{}': this node has no HTTP registry configured and no \
                     DHT client. Set `substrate.registry_url` or enable \
                     `substrate.enable_bep0044_dht`",
                    signed_info.info.service_id
                )
            });
        }

        Ok(())
    }

    /// Look up a service or substrate in the community registry.
    /// Handles both full DIDs and shorthash aliases.
    /// If `resolve` is true, it will follow service-to-substrate mappings to
    /// get mechanisms.
    pub async fn lookup(
        &self,
        id: &str,
        resolve: bool,
    ) -> Result<SignedEndpointInfo, anyhow::Error> {
        let mut result = None;

        // Try HTTP registry first
        if let Some(url) = &self.registry_url {
            let client = &self.http_client;
            let lookup_url = format!("{url}/lookup/{id}");
            tracing::debug!("Registry lookup: {}", lookup_url);

            if let Ok(response) = client.get(&lookup_url).send().await
                && response.status().is_success()
                && let Ok(info) = response.json::<SignedEndpointInfo>().await
            {
                if let Err(e) = info.verify() {
                    // FAIL FAST: Don't fall back to DHT if registry returned invalid data
                    return Err(anyhow::anyhow!("Registry returned invalid data for {id}: {e}"));
                }
                // `verify()` only proves the record is validly self-signed
                // under *its own* `service_id` -- it says nothing about
                // whether that is the DID this lookup actually asked for.
                // A compromised or malicious registry could otherwise
                // answer a lookup for A with any other party's perfectly
                // valid record, redirecting the caller. Gated on `id`
                // being a full DID -- same reason the DHT branch below
                // gates on it -- since a shorthash alias lookup cannot be
                // checked this way by construction.
                if substrate::resolve_did_key(id).is_ok() && info.info.service_id != id {
                    return Err(anyhow::anyhow!(
                        "registry returned a record for '{}', not the requested '{id}'",
                        info.info.service_id
                    ));
                }
                result = Some(info);
            }
        }

        // Try DHT if HTTP failed or wasn't configured
        let is_dht_lookup = result.is_none();
        if result.is_none()
            && let Some(dht) = &self.dht_client
        {
            // Note: DHT lookups require a public key, so shorthash aliases won't work
            // purely on DHT
            if let Ok(pubkey) = substrate::resolve_did_key(id) {
                if let Ok(pkarr_pubkey) = PublicKey::try_from(pubkey.as_bytes()) {
                    tracing::debug!("Falling back to DHT lookup for {}", id);
                    if let Some(signed_packet) = dht.resolve(&pkarr_pubkey).await {
                        if let Some(candidate) =
                            extract_verified_endpoint_from_packet(id, &signed_packet)
                        {
                            result = Some(candidate);
                        } else {
                            tracing::debug!(
                                "DHT record for {} found but no longer verifies (likely expired)",
                                id
                            );
                        }
                    }
                }
            } else {
                tracing::warn!("Cannot perform DHT lookup for non-DID identifier: {}", id);
            }
        }

        let mut info = result
            .ok_or_else(|| anyhow::anyhow!("Endpoint not found in registry or DHT for ID: {id}"))?;

        // Proactively backfill cache if it was found via DHT
        if is_dht_lookup && self.registry_url.is_some() {
            // we ignore failures on cache backfilling
            let _ = self.register(&info, false).await;
        }

        // Perform local resolution if requested
        if resolve && info.info.endpoint_type == EndpointType::Service {
            tracing::debug!("Resolving substrate mechanisms for service {}", info.info.service_id);
            let sub_info = Box::pin(self.lookup(&info.info.substrate_id, false)).await?;
            info.info.mechanisms = sub_info.info.mechanisms;
        }

        Ok(info)
    }

    /// Resolve a master anchor in the community registry or DHT.
    pub async fn resolve_master_anchor(
        &self,
        master_id: &str,
        cached_timestamp: Option<u64>,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        let mut result = None;

        // Try HTTP registry first
        if let Some(url) = &self.registry_url {
            let client = &self.http_client;
            let lookup_url = format!("{url}/lookup_master/{master_id}");
            tracing::debug!("Registry Master Anchor lookup: {}", lookup_url);

            if let Ok(response) = client.get(&lookup_url).send().await
                && response.status().is_success()
                && let Ok(body_str) = response.text().await
                && let Ok(signed_anchor) = serde_json::from_str::<SignedMasterAnchor>(&body_str)
                && signed_anchor.payload.schema == MASTER_ANCHOR_SCHEMA_V1
            {
                if signed_anchor.master_id != master_id {
                    return Err(anyhow::anyhow!(
                        "registry returned an anchor for {} when {master_id} was requested",
                        signed_anchor.master_id
                    ));
                }

                if let Err(e) = signed_anchor.verify() {
                    return Err(anyhow::anyhow!(
                        "Registry returned invalid Master Anchor for {master_id}: {e}"
                    ));
                }

                // Verification succeeded, we proceed.

                if let Some(cached) = cached_timestamp
                    && signed_anchor.payload.timestamp <= cached
                {
                    return Err(anyhow::anyhow!(
                        "Fetched Master Anchor payload is not newer than locally cached version"
                    ));
                }

                result = Some(signed_anchor.payload);
            }
        }

        // Try DHT if HTTP failed or wasn't configured
        if result.is_none()
            && let Some(dht) = &self.dht_client
            && let Ok(pubkey) = substrate::resolve_did_key(master_id)
            && let Ok(pkarr_pubkey) = PublicKey::try_from(pubkey.as_bytes())
        {
            tracing::debug!("Falling back to DHT lookup for Master Anchor {}", master_id);
            if let Some(signed_packet) = dht.resolve(&pkarr_pubkey).await {
                for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
                    if let RData::TXT(txt) = &answer.rdata
                        && let Ok(full_string) = String::try_from(txt.clone())
                        && let Ok(parsed_payload) =
                            serde_json::from_str::<MasterAnchorPayload>(&full_string)
                        && parsed_payload.schema == MASTER_ANCHOR_SCHEMA_V1
                    {
                        if let Some(cached) = cached_timestamp
                            && parsed_payload.timestamp <= cached
                        {
                            tracing::debug!(
                                "DHT returned an older or identical Master Anchor payload, \
                                 ignoring"
                            );
                            break;
                        }
                        result = Some(parsed_payload);
                        break;
                    }
                }
            }
        }

        result.ok_or_else(|| {
            anyhow::anyhow!("Master Anchor not found in registry or DHT for ID: {master_id}")
        })
    }

    /// Registers/publishes a Master Anchor payload to the DHT and optionally
    /// the HTTP registry.
    pub async fn publish_master_anchor(
        &self,
        master_id: &str,
        revoked_keys: Vec<String>,
        revoke_list_registry: Option<String>,
        identity: &Identity,
        sync_dht: bool,
    ) -> anyhow::Result<()> {
        let payload =
            MasterAnchorPayload { revoked_keys, revoke_list_registry, ..Default::default() };
        let signed_anchor = payload.sign(identity)?;
        let mut http_success = self.registry_url.is_none();

        if let Some(url) = &self.registry_url {
            let client = &self.http_client;
            let register_url = format!("{url}/register_master");
            tracing::debug!("Registry register_master_anchor: {}", register_url);

            match client.post(&register_url).json(&signed_anchor).send().await {
                Ok(response) if response.status().is_success() => {
                    http_success = true;
                }
                Ok(response) => {
                    tracing::warn!(
                        "HTTP registry returned error status for master anchor: {}",
                        response.status()
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to HTTP registry for master anchor: {}", e);
                }
            }
        }

        if !http_success {
            return Err(anyhow::anyhow!("Failed to register Master Anchor via HTTP registry"));
        }

        if let Some(dht) = &self.dht_client {
            tracing::debug!("Publishing Master Anchor to Mainline DHT (background)");
            let packet_bytes = hex::decode(&signed_anchor.pkarr_packet_hex)?;
            let bytes_obj = Bytes::from(packet_bytes);
            let pubkey = substrate::resolve_did_key(master_id)?;
            let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())?;
            let signed_packet = SignedPacket::from_relay_payload(&pkarr_pubkey, &bytes_obj)?;

            publish_dht_packet(dht.clone(), signed_packet, sync_dht, "Master Anchor").await;
        }

        Ok(())
    }

    /// The anchor currently published for `master_did`, for a holder of that
    /// master key about to republish it. `Ok(None)` means the registry has no
    /// anchor for this master; `Err` means one exists but could not be read,
    /// which a caller must not treat as "start empty."
    ///
    /// HTTP only. The DHT is a fallback for *finding* an anchor, and a master
    /// republishing its own state should not adopt a payload from a source it
    /// cannot correct.
    async fn fetch_own_master_anchor(
        &self,
        master_did: &str,
    ) -> anyhow::Result<Option<MasterAnchorPayload>> {
        let Some(url) = &self.registry_url else {
            return Ok(None);
        };

        let client = &self.http_client;
        let lookup_url = format!("{url}/lookup_master/{master_did}");
        tracing::debug!("Registry Master Anchor lookup (own): {}", lookup_url);

        let response = match client.get(&lookup_url).send().await {
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                return Ok(None);
            }
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                return Err(anyhow::anyhow!(
                    "registry returned {} fetching master anchor for {master_did}",
                    response.status()
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to connect to HTTP registry fetching master anchor for {master_did}: \
                     {e}"
                ));
            }
        };

        let body_str = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("failed to read master anchor response body: {e}"))?;
        let signed_anchor: SignedMasterAnchor = serde_json::from_str(&body_str)
            .map_err(|e| anyhow::anyhow!("failed to parse master anchor response: {e}"))?;

        if signed_anchor.master_id != master_did {
            return Err(anyhow::anyhow!(
                "registry returned an anchor for {} when {master_did} was requested",
                signed_anchor.master_id
            ));
        }

        signed_anchor.verify_signature()?;

        Ok(Some(signed_anchor.payload))
    }

    /// Publishes or refreshes `master`'s anchor, carrying forward every
    /// stateful field the current anchor holds.
    ///
    /// A `DelegationCertificate` is unusable on the wire until its master's
    /// anchor is resolvable: the destination's handshake resolves it to check
    /// revocation and fails closed when it is missing. Anchors also stop
    /// verifying after 24 hours, so this is a refresh as much as a first
    /// publish.
    ///
    /// Read-modify-write, because `publish_master_anchor` overwrites the
    /// whole payload: republishing with defaults would silently un-revoke
    /// every retired instance key of this master, and detach a delegated
    /// revocation list. `schema` and `timestamp` are re-derived by `sign` and
    /// are not carried. Races with a concurrent refresh of the same master;
    /// acceptable for an operator-run command, and what an unattended issuer
    /// has to solve properly.
    ///
    /// Reads through `fetch_own_master_anchor`, not `resolve_master_anchor`:
    /// the latter rejects an anchor older than 24 hours, and the refresh is a
    /// daily duty, so a late operator would land on that path every time and
    /// publish an empty payload over a stale but authentic one. An
    /// unreadable anchor aborts here; it never degrades to "start empty."
    pub async fn refresh_master_anchor(&self, master: &Identity) -> anyhow::Result<()> {
        let master_did = substrate::derive_did_key(&master.public_key());
        let (revoked_keys, revoke_list_registry) = self
            .fetch_own_master_anchor(&master_did)
            .await?
            .map(|prev| (prev.revoked_keys, prev.revoke_list_registry))
            .unwrap_or_default();
        // `sync_dht: false`: the HTTP publish above (awaited inside
        // `publish_master_anchor`) is the operative guarantee: resolution
        // requires a configured HTTP registry, so the DHT copy is
        // redundant, best-effort backup. This
        // is called from `roymctl`'s deploy paths (`svc deploy --master`,
        // `app deploy --mint-masters`, once per master), and blocking one of
        // those on a real mainline-DHT publish -- which can take seconds or
        // hang without connectivity -- would trade a synchronous guarantee
        // this function does not need for one it does not need either.
        self.publish_master_anchor(&master_did, revoked_keys, revoke_list_registry, master, false)
            .await
    }

    /// Adds `instance_did` to `master`'s revoked-key list and republishes
    /// the anchor. Read-modify-write over the same `fetch_own_master_anchor`
    /// path `refresh_master_anchor` uses, and for the same reason: the
    /// publish overwrites the whole payload, so anything already revoked
    /// has to be carried forward or this call un-revokes it.
    ///
    /// The value to pass is the derived **instance** DID (`temporary_did`
    /// on the certificate, what `resolve-instance-identity` computes), not
    /// the master's own -- revoking the master would repudiate every
    /// instance it has ever certified, past and future.
    ///
    /// Idempotent: revoking an already-revoked key republishes the same
    /// list rather than duplicating the entry.
    ///
    /// The read-modify-write races a concurrent refresh of the same master.
    /// Under the topology this tree supports -- mint-in-place means exactly
    /// one `MasterVault` ever holds a given master, and export/import moves
    /// a *file* rather than granting concurrent live access -- there is
    /// structurally one writer, so the race is not reachable. A redundant
    /// deployment sharing one master across two live processes would need
    /// real compare-and-set from the registry, which it does not offer.
    pub async fn revoke_instance_key(
        &self,
        master: &Identity,
        instance_did: &str,
    ) -> anyhow::Result<()> {
        let master_did = substrate::derive_did_key(&master.public_key());
        let (mut revoked_keys, revoke_list_registry) = self
            .fetch_own_master_anchor(&master_did)
            .await?
            .map(|prev| (prev.revoked_keys, prev.revoke_list_registry))
            .unwrap_or_default();
        if !revoked_keys.iter().any(|k| k == instance_did) {
            revoked_keys.push(instance_did.to_string());
        }
        self.publish_master_anchor(&master_did, revoked_keys, revoke_list_registry, master, false)
            .await
    }
}

#[async_trait::async_trait]
pub trait MasterAnchorResolver: std::fmt::Debug + Send + Sync {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error>;
}

#[async_trait::async_trait]
impl MasterAnchorResolver for RegistryClient {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.resolve_master_anchor(master_id, None).await
    }
}
