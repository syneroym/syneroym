use std::time;

use bytes::Bytes;
use pkarr::{
    Keypair, PublicKey, SignedPacket, Timestamp,
    dns::{
        CLASS, Name, ResourceRecord,
        rdata::{RData, TXT},
    },
};
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
    /// A publisher-supplied counter a *reader* compares to tell two records
    /// for the same `service_id` apart (ADR-0022 §2) -- never enforced at
    /// admission, since the registry's compare-and-swap stays
    /// last-writer-wins. `#[serde(default)]` so a record signed before this
    /// field existed deserializes as `0`, which is also the correct
    /// reading: "no generation claimed".
    #[serde(default)]
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
