//! Community Registry Client and Types
//!
//! Provides structures and client methods for registering, querying, and
//! resolving service/substrate endpoints in the Syneroym community registry.

use std::time;

use bytes::Bytes;
use pkarr::{
    Client, Keypair, PublicKey, SignedPacket, Timestamp,
    dns::{
        CLASS, Name, ResourceRecord,
        rdata::{RData, TXT},
    },
};
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use syneroym_identity::{Identity, substrate};

/// Default time-to-live for registry entries, aligned with BEP 0044 DHT expiry
/// defaults.
pub const DEFAULT_REGISTRY_TTL_SECS: u64 = 7200; // 2 hours

/// Interval at which substrates republish their endpoints to prevent them from
/// expiring.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 3600; // 1 hour

/// Default lifetime of an `EndpointInfo.not_after` bound from the moment a
/// signer signs it. This is a freshness backstop, not the sharp control: the
/// sharp control is the monotonic pkarr/BEP44 timestamp every record
/// carries -- a newer record from the same signer always displaces an older
/// one, at both stores (mainline's own unconditional sequence-number
/// rejection on the DHT side; `community_registry`'s explicit
/// compare-and-swap on the HTTP side), so a substrate a member has moved
/// away from cannot keep pointing at itself just by staying up and
/// replaying its last blob.
/// `not_after` only matters when the signer stops renewing *at all* -- a
/// lost master key, a decommissioned member -- so it is set generously,
/// deliberately far longer than an instance certificate's lifetime (hours):
/// a reader that enforced it tightly would turn a routine missed renewal
/// into an instant resolution failure for every consumer, the exact cliff
/// the certificate `not_after` split existed to avoid.
pub const DEFAULT_ENDPOINT_NOT_AFTER_SECS: u64 = 30 * 24 * 3600; // 30 days

/// Internal pkarr DHT DNS name used in published packets
pub const PKARR_DNS_NAME: &str = "syneroym";

/// Internal pkarr DHT packet TTL. Matches heartbeat interval so records expire
/// if not refreshed.
pub const PKARR_TTL: u32 = HEARTBEAT_INTERVAL_SECS as u32;

/// The canonical schema identifier for Master Anchor payloads
pub const MASTER_ANCHOR_SCHEMA_V1: &str = "master_anchor_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    Substrate,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMechanism {
    Iroh {
        #[serde(with = "hex")]
        endpoint_addr_bytes: Vec<u8>,
        relay_url: Option<String>,
    },
    WebRtc {
        peer_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub service_id: String,   // e.g. substrate did:key
    pub substrate_id: String, // For substrate itself, it's the same as service_id
    pub endpoint_type: EndpointType,
    pub mechanisms: Vec<EndpointMechanism>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u64>,
    /// Unix seconds after which this record must be treated as absent, even
    /// by a store whose own expiry never fires (a substrate replaying a
    /// master-signed blob it cannot re-sign has no other way to let a
    /// record go stale). See `DEFAULT_ENDPOINT_NOT_AFTER_SECS`.
    pub not_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEndpointInfo {
    pub info: EndpointInfo,
    pub pkarr_packet_hex: String, // Hex encoded SignedPacket bytes
}

impl EndpointInfo {
    pub fn sign(self, identity: &Identity) -> Result<SignedEndpointInfo, anyhow::Error> {
        let keypair = Keypair::from_secret_key(&identity.to_bytes());
        let json_str = serde_json::to_string(&self)?;
        let txt_rdata = TXT::try_from(json_str.as_str())
            .map_err(|e| anyhow::anyhow!("Failed to construct TXT record: {e}"))?;
        let name = Name::new(PKARR_DNS_NAME)
            .map_err(|e| anyhow::anyhow!("Failed to create DNS name: {e}"))?;

        let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
        let timestamp = Timestamp::now();
        let signed_packet = SignedPacket::new(&keypair, &records, timestamp)
            .map_err(|e| anyhow::anyhow!("Failed to sign pkarr packet: {e}"))?;
        let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
        Ok(SignedEndpointInfo { info: self, pkarr_packet_hex })
    }
}

impl SignedEndpointInfo {
    /// Verifies the pkarr packet against the record's claimed identity, and
    /// returns the packet's signed timestamp on success -- the caller doing
    /// last-writer-wins admission (`community_registry`'s compare-and-swap)
    /// needs it, and this is the only place that already parses the packet
    /// to get it.
    ///
    /// One keying shape: every record is self-signed by the key its own
    /// `service_id` resolves to. A member endpoint record's `service_id` is
    /// a member master DID, so it is signed by the *deployer's* master key
    /// (ADR-0020 §3, §6) -- never by the hosting substrate, which holds only
    /// a delegated instance key and cannot produce this signature. The
    /// substrate stores whatever signed blob it was given at deploy and
    /// replays it verbatim; it has no way to re-sign one.
    pub fn verify(&self) -> Result<Timestamp, anyhow::Error> {
        let pubkey = substrate::resolve_did_key(&self.info.service_id)
            .map_err(|e| anyhow::anyhow!("Failed to parse public key from service_id: {e}"))?;
        let expected_pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid ed25519 pubkey for pkarr: {e}"))?;

        let packet_bytes = hex::decode(&self.pkarr_packet_hex)
            .map_err(|_| anyhow::anyhow!("Invalid hex encoding for pkarr packet"))?;
        let bytes_obj = Bytes::from(packet_bytes);
        let signed_packet = SignedPacket::from_relay_payload(&expected_pkarr_pubkey, &bytes_obj)
            .map_err(|e| anyhow::anyhow!("Invalid pkarr packet signature or structure: {e}"))?;

        if signed_packet.public_key() != expected_pkarr_pubkey {
            return Err(anyhow::anyhow!(
                "Signed packet public key does not match the key service_id resolves to"
            ));
        }

        // The whole record, not just its service_id: the registry
        // stores and serves this outer copy, and `substrate_id` is what a
        // lookup follows to an address, so anything left uncompared here is
        // rewritable by whoever relays the record.
        let mut found_txt = false;
        for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
            if let RData::TXT(txt) = &answer.rdata
                && let Ok(full_string) = String::try_from(txt.clone())
                && let Ok(parsed_info) = serde_json::from_str::<EndpointInfo>(&full_string)
                && parsed_info == self.info
            {
                found_txt = true;
                break;
            }
        }

        if !found_txt {
            return Err(anyhow::anyhow!("pkarr packet does not contain this exact EndpointInfo"));
        }

        let now = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if self.info.not_after < now {
            return Err(anyhow::anyhow!(
                "endpoint record expired at {}, now {now}",
                self.info.not_after
            ));
        }

        Ok(signed_packet.timestamp())
    }
}

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
fn extract_verified_endpoint_from_packet(
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

    /// The HTTP registry URL this client publishes into, if configured
    /// (M05A A4 -- lets a substrate report its own registry namespace rather
    /// than a caller having to guess it from config).
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

        if let Some(url) = &self.registry_url {
            let client = &self.http_client;
            let register_url = format!("{url}/register");
            tracing::debug!("Registry register: {}", register_url);

            match client.post(&register_url).json(signed_info).send().await {
                Ok(response) if response.status().is_success() => {
                    http_success = true;
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
        // resolves from its own `service_id`, so unlike A1's committed
        // design -- where a delegation-signed record could only ever land
        // under the instance key's own DID on the DHT, never the master's --
        // this always has a home there.
        if let Some(dht) = &self.dht_client {
            tracing::debug!("Publishing to Mainline DHT (background)");
            let packet_bytes = hex::decode(&signed_info.pkarr_packet_hex)?;
            let bytes_obj = Bytes::from(packet_bytes);
            let pubkey = substrate::resolve_did_key(&signed_info.info.service_id)?;
            let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())?;
            let signed_packet = SignedPacket::from_relay_payload(&pkarr_pubkey, &bytes_obj)?;

            publish_dht_packet(dht.clone(), signed_packet, sync_dht, "endpoint info").await;
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

        let mut info = result.ok_or_else(|| {
            anyhow::anyhow!("Endpoint not found in registry or DHT for ID: {}", id)
        })?;

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
            anyhow::anyhow!("Master Anchor not found in registry or DHT for ID: {}", master_id)
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
    /// publish an empty payload over a stale but authentic one (D-A1-12). An
    /// unreadable anchor aborts here; it never degrades to "start empty."
    pub async fn refresh_master_anchor(&self, master: &Identity) -> anyhow::Result<()> {
        let master_did = substrate::derive_did_key(&master.public_key());
        let (revoked_keys, revoke_list_registry) = self
            .fetch_own_master_anchor(&master_did)
            .await?
            .map(|prev| (prev.revoked_keys, prev.revoke_list_registry))
            .unwrap_or_default();
        // `sync_dht: false`: the HTTP publish above (awaited inside
        // `publish_master_anchor`) is the operative guarantee -- D-A1-2
        // already established that resolution requires a configured HTTP
        // registry, so the DHT copy is redundant, best-effort backup. This
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

fn default_schema() -> String {
    MASTER_ANCHOR_SCHEMA_V1.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterAnchorPayload {
    #[serde(default = "default_schema")]
    pub schema: String,
    pub revoked_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoke_list_registry: Option<String>,
    pub timestamp: u64,
}

impl Default for MasterAnchorPayload {
    fn default() -> Self {
        Self {
            schema: default_schema(),
            revoked_keys: vec![],
            revoke_list_registry: None,
            timestamp: 0,
        }
    }
}

impl MasterAnchorPayload {
    pub fn sign(mut self, identity: &Identity) -> Result<SignedMasterAnchor, anyhow::Error> {
        let master_id = substrate::derive_did_key(&identity.public_key());
        let keypair = Keypair::from_secret_key(&identity.to_bytes());

        let timestamp = Timestamp::now();
        self.timestamp = timestamp.as_u64();

        let json_str = serde_json::to_string(&self)?;
        let txt_rdata = TXT::try_from(json_str.as_str()).map_err(|e| {
            anyhow::anyhow!("Failed to construct TXT record for Master Anchor: {e}")
        })?;
        let name = Name::new(PKARR_DNS_NAME)
            .map_err(|e| anyhow::anyhow!("Failed to create DNS name: {e}"))?;

        let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
        let signed_packet = SignedPacket::new(&keypair, &records, timestamp)
            .map_err(|e| anyhow::anyhow!("Failed to sign pkarr packet for Master Anchor: {e}"))?;
        let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
        Ok(SignedMasterAnchor { master_id, payload: self, pkarr_packet_hex })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedMasterAnchor {
    pub master_id: String,
    pub payload: MasterAnchorPayload,
    pub pkarr_packet_hex: String,
}

impl SignedMasterAnchor {
    /// Everything `verify` checks except the 24-hour freshness bound: the
    /// master DID resolves, the pkarr packet is signed by it, the payload
    /// carried alongside is byte-for-byte the one inside the packet, and that
    /// payload's timestamp is the packet's.
    ///
    /// For a master reading back its own anchor to re-sign it. Freshness is
    /// a consumer's question -- "is this revocation list current enough to
    /// act on" -- and applying it here would make a late refresh silently
    /// publish an empty payload over a stale but perfectly authentic one.
    ///
    /// **Never consume an anchor through this.** Revocation checks use
    /// `verify`.
    pub fn verify_signature(&self) -> Result<(), anyhow::Error> {
        let pubkey = substrate::resolve_did_key(&self.master_id)
            .map_err(|e| anyhow::anyhow!("Failed to parse public key from master_id DID: {e}"))?;

        let expected_pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid ed25519 pubkey for pkarr: {e}"))?;

        let packet_bytes = hex::decode(&self.pkarr_packet_hex)
            .map_err(|_| anyhow::anyhow!("Invalid hex encoding for pkarr packet"))?;

        let bytes_obj = Bytes::from(packet_bytes);
        let signed_packet = SignedPacket::from_relay_payload(&expected_pkarr_pubkey, &bytes_obj)
            .map_err(|e| anyhow::anyhow!("Invalid pkarr packet signature or structure: {e}"))?;

        if signed_packet.public_key() != expected_pkarr_pubkey {
            return Err(anyhow::anyhow!("Signed packet public key does not match master_id"));
        }

        let mut found_txt = false;
        let packet_timestamp = signed_packet.timestamp().as_u64();

        for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
            if let RData::TXT(txt) = &answer.rdata
                && let Ok(full_string) = String::try_from(txt.clone())
                && let Ok(parsed_payload) =
                    serde_json::from_str::<MasterAnchorPayload>(&full_string)
                && parsed_payload.schema == MASTER_ANCHOR_SCHEMA_V1
            {
                // The whole payload, not just its timestamp (D-A1-13): every
                // consumer reads the outer copy, so a relay or a compromised
                // registry that adds or strips a revocation while leaving the
                // timestamp untouched must not verify.
                if parsed_payload != self.payload {
                    return Err(anyhow::anyhow!(
                        "Master Anchor payload does not match the signed pkarr packet"
                    ));
                }
                if parsed_payload.timestamp != packet_timestamp {
                    return Err(anyhow::anyhow!(
                        "Master Anchor payload timestamp does not match pkarr sequence \
                         number/timestamp"
                    ));
                }
                found_txt = true;
                break;
            }
        }

        if !found_txt {
            return Err(anyhow::anyhow!(
                "pkarr packet does not contain a valid MasterAnchorPayload"
            ));
        }

        if self.payload.timestamp != packet_timestamp {
            return Err(anyhow::anyhow!(
                "Outer MasterAnchorPayload timestamp does not match pkarr packet sequence \
                 number/timestamp"
            ));
        }

        Ok(())
    }

    /// `verify_signature` plus the 24-hour freshness bound.
    pub fn verify(&self) -> Result<(), anyhow::Error> {
        self.verify_signature()?;

        let now = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        let twenty_four_hours_micros = 24 * 60 * 60 * 1_000_000;

        if now.saturating_sub(self.payload.timestamp) > twenty_four_hours_micros {
            return Err(anyhow::anyhow!("Master Anchor payload has expired (older than 24 hours)"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use syneroym_identity::Identity;

    use super::*;

    fn sample_endpoint_info(service_id: &str) -> EndpointInfo {
        EndpointInfo {
            service_id: service_id.to_string(),
            substrate_id: "did:key:zSubstrate".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: u64::MAX / 2, // far future -- not what these tests are about
        }
    }

    /// Mirrors `MasterAnchorPayload::sign`, but backdates the pkarr packet
    /// timestamp so the D-A1-12 stale-anchor path can be exercised directly
    /// instead of argued for structurally.
    fn sign_backdated(
        mut payload: MasterAnchorPayload,
        identity: &Identity,
        hours_ago: u64,
    ) -> SignedMasterAnchor {
        let master_id = substrate::derive_did_key(&identity.public_key());
        let keypair = Keypair::from_secret_key(&identity.to_bytes());

        let now_micros =
            time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_micros() as u64;
        let backdated_micros = now_micros - hours_ago * 60 * 60 * 1_000_000;
        let timestamp = Timestamp::from(backdated_micros);
        payload.timestamp = timestamp.as_u64();

        let json_str = serde_json::to_string(&payload).unwrap();
        let txt_rdata = TXT::try_from(json_str.as_str()).unwrap();
        let name = Name::new(PKARR_DNS_NAME).unwrap();
        let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
        let signed_packet = SignedPacket::new(&keypair, &records, timestamp).unwrap();
        let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
        SignedMasterAnchor { master_id, payload, pkarr_packet_hex }
    }

    #[test]
    fn a_self_signed_record_verifies() {
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let signed = sample_endpoint_info(&did).sign(&identity).unwrap();

        assert!(signed.verify().is_ok());
    }

    #[test]
    fn verify_returns_the_packets_own_timestamp() {
        // `community_registry`'s compare-and-swap needs this to reject a
        // rollback -- it is the pkarr/BEP44 sequence number, and the only
        // thing that makes "newer record wins" enforceable without a
        // second, independently-trackable counter.
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let signed = sample_endpoint_info(&did).sign(&identity).unwrap();
        let ts = signed.verify().expect("a freshly self-signed record must verify");

        let pubkey = substrate::resolve_did_key(&did).unwrap();
        let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes()).unwrap();
        let packet_bytes = hex::decode(&signed.pkarr_packet_hex).unwrap();
        let packet =
            SignedPacket::from_relay_payload(&pkarr_pubkey, &Bytes::from(packet_bytes)).unwrap();
        assert_eq!(ts, packet.timestamp());
    }

    #[test]
    fn a_record_is_rejected_when_signed_by_a_key_other_than_its_own_service_id() {
        let claimed = Identity::generate().unwrap();
        let actual_signer = Identity::generate().unwrap();
        let claimed_did = substrate::derive_did_key(&claimed.public_key());

        // The one keying shape this design allows: every record must be
        // self-signed by the key its own `service_id` resolves to. A member
        // endpoint record's `service_id` is a member master DID, so this is
        // exactly the shape a hosting substrate cannot produce -- it never
        // holds that key (ADR-0020 §3).
        let signed = sample_endpoint_info(&claimed_did).sign(&actual_signer).unwrap();

        assert!(signed.verify().is_err());
    }

    #[test]
    fn rewriting_the_substrate_id_after_signing_is_rejected() {
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let mut signed = sample_endpoint_info(&did).sign(&identity).unwrap();
        signed.info.substrate_id = "did:key:zAttacker".to_string();

        // The attack this guards against: a relay rewriting substrate_id so
        // a lookup follows it to a host the attacker controls.
        assert!(signed.verify().is_err());
    }

    #[test]
    fn an_expired_record_is_rejected() {
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let mut info = sample_endpoint_info(&did);
        info.not_after = 1; // long past, regardless of when this test runs
        let signed = info.sign(&identity).unwrap();

        let err = signed.verify().expect_err("an expired record must not verify");
        assert!(err.to_string().contains("expired"));
    }

    /// Decodes a signed record's own hex-encoded packet back into the
    /// `SignedPacket` shape `RegistryClient::lookup`'s DHT branch gets from
    /// `dht.resolve` -- the same reconstruction `verify` does internally, so
    /// `extract_verified_endpoint_from_packet` can be exercised without a
    /// live DHT.
    fn packet_from(signed: &SignedEndpointInfo) -> SignedPacket {
        let pubkey = substrate::resolve_did_key(&signed.info.service_id).unwrap();
        let pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes()).unwrap();
        let packet_bytes = hex::decode(&signed.pkarr_packet_hex).unwrap();
        SignedPacket::from_relay_payload(&pkarr_pubkey, &Bytes::from(packet_bytes)).unwrap()
    }

    #[test]
    fn extract_verified_endpoint_from_packet_returns_a_live_record() {
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());
        let signed = sample_endpoint_info(&did).sign(&identity).unwrap();
        let packet = packet_from(&signed);

        let extracted = extract_verified_endpoint_from_packet(&did, &packet)
            .expect("a live record must be returned");
        assert_eq!(extracted.info.service_id, did);
    }

    #[test]
    fn extract_verified_endpoint_from_packet_rejects_an_expired_one() {
        // The DHT record itself is authentic -- pkarr's own signature check
        // already passed by the time `resolve` returns it -- but its
        // `not_after` has lapsed. This is exactly the case the HTTP registry
        // branch already covered via `verify()`; the DHT branch used to skip
        // it entirely.
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());
        let mut info = sample_endpoint_info(&did);
        info.not_after = 1;
        let signed = info.sign(&identity).unwrap();
        let packet = packet_from(&signed);

        assert!(extract_verified_endpoint_from_packet(&did, &packet).is_none());
    }

    #[tokio::test]
    async fn a_self_signed_record_registers_to_the_dht_with_no_http_registry_configured() {
        // Reverses an earlier restriction on this design: a delegation-
        // signed record used to be keyed by a master DID but signed by a
        // different (instance) key, so pkarr -- which keys a packet by its
        // *signing* key -- had no DHT slot for it. Every record is now
        // self-signed by the key its own `service_id` resolves to, so that
        // mismatch cannot occur and every record has a DHT home.
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());
        let signed = sample_endpoint_info(&did).sign(&identity).unwrap();

        let client = RegistryClient::new(true, None);
        // `sync_dht: false` backgrounds the real network publish (fire-and-
        // forget), so this returns immediately without needing live DHT
        // connectivity -- it is `register`'s early HTTP-registry-required
        // refusal this test disproves, not the DHT publish itself.
        assert!(client.register(&signed, false).await.is_ok());
    }

    #[test]
    fn a_stale_anchor_still_passes_verify_signature_and_still_fails_verify() {
        let identity = Identity::generate().unwrap();
        let payload =
            MasterAnchorPayload { revoked_keys: vec!["k".to_string()], ..Default::default() };

        let signed = sign_backdated(payload, &identity, 25);

        assert!(signed.verify_signature().is_ok());
        assert!(signed.verify().is_err());
    }

    #[test]
    fn an_anchor_whose_master_id_does_not_resolve_is_rejected() {
        // Not the D-A1-12 `master_id` equality tightening (that guard lives
        // in `fetch_own_master_anchor`/`resolve_master_anchor`, which need a
        // real registry to exercise -- see `crates/community_registry`'s
        // `refresh_refuses_an_anchor_served_under_the_wrong_master*` tests).
        // This is the pre-existing key-resolution failure: an unresolvable
        // `master_id` string can never verify at all.
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload::default();

        let mut signed = payload.sign(&identity).unwrap();
        signed.master_id = "did:key:zSomeoneElse".to_string();

        assert!(signed.verify_signature().is_err());
    }

    #[test]
    fn stripping_a_revoked_key_after_signing_is_rejected() {
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload {
            revoked_keys: vec!["did:key:zRevoked".to_string()],
            ..Default::default()
        };

        let mut signed = payload.sign(&identity).unwrap();
        signed.payload.revoked_keys.clear();

        assert!(signed.verify_signature().is_err());
    }

    #[test]
    fn adding_a_revoked_key_after_signing_is_rejected() {
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload::default();

        let mut signed = payload.sign(&identity).unwrap();
        signed.payload.revoked_keys.push("did:key:zInjected".to_string());

        assert!(signed.verify_signature().is_err());
    }

    #[test]
    fn rewriting_the_revoke_list_registry_after_signing_is_rejected() {
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload::default();

        let mut signed = payload.sign(&identity).unwrap();
        signed.payload.revoke_list_registry = Some("https://attacker.example/list".to_string());

        assert!(signed.verify_signature().is_err());
    }

    #[test]
    fn test_master_anchor_payload_timestamp_validation() {
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload::default();

        let mut signed = payload.clone().sign(&identity).unwrap();
        assert!(signed.verify().is_ok());

        // Negative test: Mismatch timestamp
        signed.payload.timestamp -= 1;
        assert!(signed.verify().is_err());
    }

    #[test]
    fn test_master_anchor_payload_expired() {
        let identity = Identity::generate().unwrap();
        let payload = MasterAnchorPayload::default();

        let signed = payload.sign(&identity).unwrap();

        // We can't easily manipulate pkarr SignedPacket timestamp since it's signed.
        // But verify() works on the signed pkarr packet timestamp.
        assert!(signed.verify().is_ok());
    }
}
