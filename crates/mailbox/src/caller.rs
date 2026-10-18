//! Proven caller identities and certificate verification for the mailbox.

use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use syneroym_core::dht_registry::MasterAnchorResolver;
use syneroym_identity::{DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE, substrate};
use syneroym_rpc::{AuthLevel, CallerContext};
use tokio::time;

use crate::error::MailboxError;

/// A service instance proven by the transport (ADR-0016): the member master
/// it acts for, its instance key DID, and when its certificate was issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instance {
    pub master_did: String,
    pub instance_did: String,
    pub issued_at_secs: u64,
}

/// Extracts and verifies the service instance identity from `caller`.
///
/// The router has already made these checks for a wire call; a same-node call
/// reaches the service with no handshake, so they are made again here.
pub(crate) fn instance_of(caller: &CallerContext, now_secs: u64) -> Result<Instance, MailboxError> {
    if !matches!(caller.auth, AuthLevel::Delegated | AuthLevel::Ucan) {
        return Err(MailboxError::Unauthenticated);
    }
    let proof = caller
        .proof
        .as_ref()
        .ok_or_else(|| MailboxError::NotAnInstance("no certificate".to_string()))?;
    let delegation_json = proof
        .delegation_json
        .as_ref()
        .ok_or_else(|| MailboxError::NotAnInstance("no certificate".to_string()))?;
    let cert = DelegationCertificate::from_json(delegation_json)
        .map_err(|_| MailboxError::NotAnInstance("unreadable certificate".to_string()))?;

    if cert.scope != SCOPE_SERVICE_INSTANCE {
        return Err(MailboxError::NotAnInstance("not a service-instance certificate".to_string()));
    }
    if cert.master_did != caller.caller_did {
        return Err(MailboxError::NotAnInstance("certificate is for another master".to_string()));
    }

    let pubkey_bytes = hex::decode(&proof.pubkey_hex)
        .map_err(|e| MailboxError::NotAnInstance(format!("invalid public key hex: {e}")))?;
    let array: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| MailboxError::NotAnInstance("invalid public key length".to_string()))?;
    let verifying_key = VerifyingKey::from_bytes(&array)
        .map_err(|e| MailboxError::NotAnInstance(format!("invalid public key: {e}")))?;
    let key_did = substrate::derive_did_key(&verifying_key);

    if key_did != cert.temporary_did {
        return Err(MailboxError::NotAnInstance("key does not match the certificate".to_string()));
    }

    cert.verify_at(&cert.master_did, &[SCOPE_SERVICE_INSTANCE], now_secs)
        .map_err(|e| MailboxError::NotAnInstance(e.to_string()))?;

    Ok(Instance {
        master_did: cert.master_did,
        instance_did: cert.temporary_did,
        issued_at_secs: cert.issued_at_secs,
    })
}

/// Checks that the instance key is not revoked according to the master's
/// anchor.
///
/// The router has already made this check for a wire call; a same-node call
/// reaches the service with no handshake, so it is made again here.
pub(crate) async fn check_not_revoked(
    resolver: &dyn MasterAnchorResolver,
    who: &Instance,
) -> Result<(), MailboxError> {
    let anchor_res =
        time::timeout(Duration::from_secs(5), resolver.resolve_master_anchor(&who.master_did))
            .await;

    let anchor = match anchor_res {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            return Err(MailboxError::Internal(format!("could not read the master anchor: {e}")));
        }
        Err(_) => {
            return Err(MailboxError::Internal(
                "could not read the master anchor: timeout".to_string(),
            ));
        }
    };

    if anchor.revoked_keys.contains(&who.instance_did) {
        return Err(MailboxError::NotAnInstance("the instance key is revoked".to_string()));
    }

    Ok(())
}

/// Extracts and verifies the sender identity from `caller`.
///
/// Senders must present a service-instance certificate proving their master
/// DID.
pub(crate) fn sender_of(caller: &CallerContext, now_secs: u64) -> Result<Instance, MailboxError> {
    instance_of(caller, now_secs)
}

#[cfg(test)]
mod tests;
