//! One record-signing certificate per service, held in that service's own
//! encrypted storage.
//!
//! The certificate carries no private key, and it certifies exactly one
//! key -- this service's own -- so it is useless anywhere else. The
//! signing host still takes it as a parameter on every call; this module
//! is only where the guest keeps that parameter between the ceremony that
//! mints it and the call that uses it.

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{CollectionSchema, RecordWriteValue},
        signing::{Principal, RecordDraft},
    },
};
pub use syneroym_signed_record::{DelegationCertificate, SCOPE_RECORD_SIGNING};

use crate::{
    backup::{BUNDLE_MANIFEST_VERSION, Bundle},
    record,
};

pub mod verbs;
pub use verbs::{
    InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
    install_signing_certificate, signing_status,
};

pub const CERTIFICATES: &str = "signing_certificates";
/// One row, always. A second person on one installation is out of scope
/// for the first release, and a fixed id makes that visible rather than
/// letting a second row appear unnoticed.
pub const CERTIFICATE_ID: &str = "current";
pub const MAX_CERTIFICATE_LIFETIME_SECS: u64 = 5 * 365 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCertificate {
    /// The `did:key` this certificate certifies -- this service's own
    /// signing key as it stood when the certificate was installed.
    pub signing_did: String,
    pub master_did: String,
    pub issued_at_secs: u64,
    pub expires_at_secs: u64,
    /// The certificate JSON exactly as minted, handed to the signing host
    /// unchanged.
    pub certificate: String,
}

/// `Stale` is a real state, not a theoretical one: the signing key is
/// re-derived from the recorded service owner on every call, so changing
/// a service's owner re-keys it and every stored certificate stops
/// matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum CertificateStatus {
    Missing,
    Stale { installed_for: String, current: String },
    Expired { expires_at_secs: u64 },
    Installed { master_did: String, expires_at_secs: u64, near_expiry: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    #[error("signing is not enrolled on this service")]
    NotEnrolled,
    #[error("the installed certificate expired at {0}")]
    Expired(u64),
    #[error(
        "the installed certificate certifies '{installed_for}', but this service now signs with \
         '{current}'"
    )]
    Stale { installed_for: String, current: String },
    #[error("certificate rejected: {0}")]
    Rejected(String),
    #[error("this installation has no recorded owner, so it cannot sign as anyone")]
    NoOwner,
    #[error("storage: {0}")]
    Storage(String),
}

pub async fn install<H: AppHost>(
    host: &H,
    certificate_json: &str,
    now_secs: u64,
) -> Result<StoredCertificate, CertificateError> {
    if certificate_json.len() > 16 * 1024 {
        return Err(CertificateError::Rejected("too large".to_string()));
    }
    let cert = DelegationCertificate::from_json(certificate_json)
        .map_err(|e| CertificateError::Rejected(e.to_string()))?;

    let id = AppSigning::signing_identity(host)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    let owner = id.owner_did.ok_or(CertificateError::NoOwner)?;

    if cert.temporary_did != id.signing_did {
        return Err(CertificateError::Rejected(format!(
            "certifies {}, not this service's signing key {}",
            cert.temporary_did, id.signing_did
        )));
    }
    if cert.master_did != owner {
        return Err(CertificateError::Rejected(format!(
            "names master {}, this installation's owner is {}",
            cert.master_did, owner
        )));
    }
    cert.verify_chain_at(&cert.master_did, &[SCOPE_RECORD_SIGNING], now_secs)
        .map_err(|e| CertificateError::Rejected(e.to_string()))?;

    if now_secs >= cert.expires_at_secs {
        return Err(CertificateError::Rejected(format!(
            "already expired at {}",
            cert.expires_at_secs
        )));
    }
    if cert.expires_at_secs.saturating_sub(now_secs) > MAX_CERTIFICATE_LIFETIME_SECS {
        return Err(CertificateError::Rejected(format!(
            "lifetime exceeds maximum allowed ({MAX_CERTIFICATE_LIFETIME_SECS} seconds)"
        )));
    }

    ensure_collection(host, CERTIFICATES).await.map_err(CertificateError::Storage)?;

    let stored = StoredCertificate {
        signing_did: id.signing_did,
        master_did: owner,
        issued_at_secs: cert.issued_at_secs,
        expires_at_secs: cert.expires_at_secs,
        certificate: certificate_json.to_string(),
    };

    let payload =
        serde_json::to_vec(&stored).map_err(|e| CertificateError::Storage(e.to_string()))?;
    AppDataLayer::put(
        host,
        CERTIFICATES.to_string(),
        RecordWriteValue { id: CERTIFICATE_ID.to_string(), payload },
    )
    .await
    .map_err(|e| CertificateError::Storage(e.to_string()))?;

    Ok(stored)
}

/// Reads and deserialises the single stored certificate row, if any.
/// Ensures the collection exists as a side-effect, matching the behaviour
/// callers relied on before this helper existed.
async fn load_certificate<H: AppHost>(
    host: &H,
) -> Result<Option<StoredCertificate>, CertificateError> {
    ensure_collection(host, CERTIFICATES).await.map_err(CertificateError::Storage)?;
    let row = AppDataLayer::get(host, CERTIFICATES.to_string(), CERTIFICATE_ID.to_string())
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    let Some(row) = row else { return Ok(None) };
    let stored: StoredCertificate = serde_json::from_slice(&row.payload)
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    Ok(Some(stored))
}

pub async fn status<H: AppHost>(
    host: &H,
    now_secs: u64,
) -> Result<CertificateStatus, CertificateError> {
    let Some(stored) = load_certificate(host).await? else {
        return Ok(CertificateStatus::Missing);
    };
    let id = AppSigning::signing_identity(host)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    if stored.signing_did != id.signing_did {
        return Ok(CertificateStatus::Stale {
            installed_for: stored.signing_did,
            current: id.signing_did,
        });
    }
    if now_secs >= stored.expires_at_secs {
        return Ok(CertificateStatus::Expired { expires_at_secs: stored.expires_at_secs });
    }
    let near_expiry = stored.expires_at_secs.saturating_sub(now_secs) < 6 * 3600;
    Ok(CertificateStatus::Installed {
        master_did: stored.master_did,
        expires_at_secs: stored.expires_at_secs,
        near_expiry,
    })
}

pub async fn person_principal<H: AppHost>(
    host: &H,
    now_secs: u64,
) -> Result<(Principal, String), CertificateError> {
    let Some(stored) = load_certificate(host).await? else {
        return Err(CertificateError::NotEnrolled);
    };
    let id = AppSigning::signing_identity(host)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    if stored.signing_did != id.signing_did {
        return Err(CertificateError::Stale {
            installed_for: stored.signing_did,
            current: id.signing_did,
        });
    }
    if now_secs >= stored.expires_at_secs {
        return Err(CertificateError::Expired(stored.expires_at_secs));
    }
    Ok((Principal::Delegated(stored.certificate), stored.master_did))
}

/// Signs `bundle.manifest` as the person and stores the envelope in
/// `bundle.manifest_signature`. Refuses with the same `CertificateError`
/// words every signing verb uses when this service is not enrolled.
pub async fn sign_bundle<H: AppHost>(
    host: &H,
    bundle: &mut Bundle,
    now_secs: u64,
) -> Result<(), CertificateError> {
    let (principal, _owner) = person_principal(host, now_secs).await?;
    let payload =
        bundle.manifest_payload().map_err(|e| CertificateError::Storage(e.to_string()))?;
    let draft = RecordDraft {
        version: BUNDLE_MANIFEST_VERSION,
        record_type: record::RECORD_BUNDLE_MANIFEST.to_string(),
        subject: bundle.manifest.subject_did.clone(),
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let signed = AppSigning::sign_record(host, draft, principal)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    bundle.manifest_signature = Some(signed);
    Ok(())
}

pub async fn owner_did<H: AppHost>(host: &H) -> Result<String, CertificateError> {
    let id = AppSigning::signing_identity(host)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    id.owner_did.ok_or(CertificateError::NoOwner)
}

pub async fn ensure_collection<H: AppHost>(host: &H, name: &str) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: vec![], text_fields: vec![] },
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use syneroym_app_host::types::signing::SigningIdentity;
    use syneroym_identity::{Identity, substrate};
    pub use syneroym_signed_record::{DelegationCertificate, SCOPE_RECORD_SIGNING};

    use super::*;
    pub(crate) use crate::testing::TestHost;

    #[tokio::test]
    async fn test_certificate_status_and_install_flow() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());

        let service_key = Identity::generate().unwrap();
        let service_did = substrate::derive_did_key(&service_key.public_key());

        let host = TestHost::default();
        *host.signing_id.lock().unwrap() = Some(SigningIdentity {
            signing_did: service_did.clone(),
            pubkey_hex: "00".to_string(),
            owner_did: Some(master_did.clone()),
        });

        // 2. Mint valid cert
        let cert = DelegationCertificate::issue(
            &master,
            service_key.public_key(),
            720 * 3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();
        let cert_json = cert.to_json().unwrap();
        let now = cert.issued_at_secs;

        // 1. Initial status: Missing
        assert_eq!(status(&host, now).await.unwrap(), CertificateStatus::Missing);

        // 3. Install valid cert
        let stored = install(&host, &cert_json, now).await.unwrap();
        assert_eq!(stored.master_did, master_did);

        // 4. Status is now Installed
        match status(&host, now).await.unwrap() {
            CertificateStatus::Installed { master_did: m, near_expiry: false, .. } => {
                assert_eq!(m, master_did);
            }
            other => panic!("expected Installed, got {other:?}"),
        }

        // 5. Test Expired status
        match status(&host, cert.expires_at_secs + 1).await.unwrap() {
            CertificateStatus::Expired { expires_at_secs } => {
                assert_eq!(expires_at_secs, cert.expires_at_secs);
            }
            other => panic!("expected Expired, got {other:?}"),
        }

        // 6. Test Stale status when service_did changes
        *host.signing_id.lock().unwrap() = Some(SigningIdentity {
            signing_did: "did:key:z6Mnewservicekey".to_string(),
            pubkey_hex: "00".to_string(),
            owner_did: Some(master_did.clone()),
        });
        match status(&host, 1000).await.unwrap() {
            CertificateStatus::Stale { installed_for, current } => {
                assert_eq!(installed_for, service_did);
                assert_eq!(current, "did:key:z6Mnewservicekey");
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }
}
