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

use super::types::{PKARR_DNS_NAME, PKARR_TTL};

/// The canonical schema identifier for Master Anchor payloads
pub const MASTER_ANCHOR_SCHEMA_V1: &str = "master_anchor_v1";

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
                // The whole payload, not just its timestamp: every
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
