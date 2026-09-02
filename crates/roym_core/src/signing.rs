//! One record-signing certificate per service, held in that service's own
//! encrypted storage.
//!
//! The certificate carries no private key, and it certifies exactly one
//! key -- this service's own -- so it is useless anywhere else. The
//! signing host still takes it as a parameter on every call; this module
//! is only where the guest keeps that parameter between the ceremony that
//! mints it and the call that uses it.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{CollectionSchema, RecordWriteValue},
        signing::Principal,
    },
};
use syneroym_signed_record::{DelegationCertificate, SCOPE_RECORD_SIGNING};

use crate::{
    clock,
    envelope::{Request, Response},
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
            "lifetime exceeds maximum allowed ({} seconds)",
            MAX_CERTIFICATE_LIFETIME_SECS
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

pub async fn owner_did<H: AppHost>(host: &H) -> Result<String, CertificateError> {
    let id = AppSigning::signing_identity(host)
        .await
        .map_err(|e| CertificateError::Storage(e.to_string()))?;
    id.owner_did.ok_or(CertificateError::NoOwner)
}

pub async fn ensure_collection<H: AppHost>(host: &H, name: &str) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: vec![] },
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn handle_certificate_verb<H: AppHost>(
    host: &H,
    prefix: &str,
    req: &Request,
) -> Option<Response> {
    let suffix = req.method.strip_prefix(prefix)?;
    match suffix {
        "signing-status" => {
            let id = match AppSigning::signing_identity(host).await {
                Ok(i) => i,
                Err(e) => return Some(Response::internal_error(e.to_string())),
            };
            match status(host, clock::now_secs()).await {
                Ok(s) => Some(Response::ok(json!({
                    "signing_did": id.signing_did,
                    "pubkey_hex": id.pubkey_hex,
                    "owner_did": id.owner_did,
                    "certificate": s
                }))),
                Err(e) => Some(Response::internal_error(e.to_string())),
            }
        }
        "install-signing-certificate" => {
            let cert = match req.params.get("certificate").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => return Some(Response::invalid_params("certificate is required")),
            };
            match install(host, cert, clock::now_secs()).await {
                Ok(s) => Some(Response::ok(json!({
                    "master_did": s.master_did,
                    "expires_at_secs": s.expires_at_secs
                }))),
                Err(CertificateError::Rejected(m)) => Some(Response::invalid_params(m)),
                Err(e) => Some(Response::internal_error(e.to_string())),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use syneroym_app_host::{
        AppAppConfig, AppBlobStore, AppConversation, AppDataLayer, AppInvocation, AppMessaging,
        AppProxy, AppSigning, AppVault, AppWebSocket,
        types::{
            app_config::ConfigError,
            blob_store::BlobError,
            conversation::{
                ConversationError, ConversationSummary, DeliveryState, HistoryPage,
                MembershipEvent, Message,
            },
            data_layer::{
                CollectionSchema, DataLayerError, Mutation, QueryOptions, QueryResult,
                RecordReadValue,
            },
            http::FrameKind,
            invocation::CallerOrigin,
            messaging::MessagingError,
            proxy::{CallOptions, CallTarget, ProxyError},
            signing::{RecordDraft, SigningError, SigningIdentity},
            vault::VaultError,
        },
    };
    use syneroym_identity::{Identity, substrate};
    use syneroym_signed_record::{DelegationCertificate, SCOPE_RECORD_SIGNING};

    use super::*;

    #[derive(Default)]
    struct TestHost {
        storage: Mutex<HashMap<String, HashMap<String, Vec<u8>>>>,
        signing_id: Mutex<Option<SigningIdentity>>,
        /// What `AppInvocation::caller` reports. `None` reads as
        /// `CallerOrigin::Internal`, the ordinary local-dispatch answer.
        caller_origin: Mutex<Option<CallerOrigin>>,
    }

    impl TestHost {
        #[allow(dead_code)] // used by `admit`'s tests once they share this stub
        fn set_caller_origin(&self, origin: CallerOrigin) {
            *self.caller_origin.lock().unwrap() = Some(origin);
        }
    }

    impl AppInvocation for TestHost {
        async fn caller(&self) -> CallerOrigin {
            self.caller_origin.lock().unwrap().clone().unwrap_or(CallerOrigin::Internal)
        }
    }

    impl AppDataLayer for TestHost {
        async fn create_collection(&self, schema: CollectionSchema) -> Result<(), DataLayerError> {
            let mut store = self.storage.lock().unwrap();
            store.entry(schema.name).or_default();
            Ok(())
        }
        async fn drop_collection(&self, name: String) -> Result<(), DataLayerError> {
            let mut store = self.storage.lock().unwrap();
            store.remove(&name);
            Ok(())
        }
        async fn put(
            &self,
            collection: String,
            value: RecordWriteValue,
        ) -> Result<(), DataLayerError> {
            let mut store = self.storage.lock().unwrap();
            store.entry(collection).or_default().insert(value.id, value.payload);
            Ok(())
        }
        async fn patch(
            &self,
            _col: String,
            _id: String,
            _patch: Vec<u8>,
        ) -> Result<(), DataLayerError> {
            unimplemented!()
        }
        async fn get(
            &self,
            collection: String,
            id: String,
        ) -> Result<Option<RecordReadValue>, DataLayerError> {
            let store = self.storage.lock().unwrap();
            if let Some(col) = store.get(&collection)
                && let Some(val) = col.get(&id)
            {
                return Ok(Some(RecordReadValue {
                    id,
                    payload: val.clone(),
                    creator_id: String::new(),
                    created_at: 0,
                    updated_at: 0,
                }));
            }
            Ok(None)
        }
        async fn query(
            &self,
            _col: String,
            _opts: QueryOptions,
        ) -> Result<QueryResult, DataLayerError> {
            unimplemented!()
        }
        async fn aggregate(
            &self,
            _col: String,
            _pipe: String,
        ) -> Result<syneroym_app_host::types::data_layer::RawQueryResult, DataLayerError> {
            unimplemented!()
        }
        async fn delete(&self, _col: String, _id: String) -> Result<(), DataLayerError> {
            unimplemented!()
        }
        async fn delete_many(&self, _col: String, _filter: String) -> Result<u64, DataLayerError> {
            unimplemented!()
        }
        async fn batch_mutate(
            &self,
            _col: String,
            _muts: Vec<Mutation>,
        ) -> Result<(), DataLayerError> {
            unimplemented!()
        }
        async fn execute_ddl(&self, _sql: String) -> Result<(), DataLayerError> {
            unimplemented!()
        }
        async fn query_raw(
            &self,
            _sql: String,
            _params: Vec<syneroym_app_host::types::data_layer::SqlValue>,
        ) -> Result<syneroym_app_host::types::data_layer::RawQueryResult, DataLayerError> {
            unimplemented!()
        }
        async fn check_access(
            &self,
            _col: String,
            _id: String,
            _op: String,
        ) -> Result<bool, DataLayerError> {
            unimplemented!()
        }
    }

    struct DummyWriter;
    impl syneroym_app_host::AppBlobWriter for DummyWriter {
        async fn write(&mut self, _chunk: Vec<u8>) -> Result<(), BlobError> {
            unimplemented!()
        }
        async fn finish(self) -> Result<String, BlobError> {
            unimplemented!()
        }
        async fn abort(self) {}
    }

    struct DummyReader;
    impl syneroym_app_host::AppBlobReader for DummyReader {
        async fn read(&mut self, _max_bytes: u32) -> Result<Vec<u8>, BlobError> {
            unimplemented!()
        }
    }

    impl AppBlobStore for TestHost {
        type Writer = DummyWriter;
        type Reader = DummyReader;
        async fn put_blob(&self, _data: Vec<u8>) -> Result<String, BlobError> {
            unimplemented!()
        }
        async fn get_blob(&self, _hash: String) -> Result<Vec<u8>, BlobError> {
            unimplemented!()
        }
        async fn open_upload(&self) -> Result<DummyWriter, BlobError> {
            unimplemented!()
        }
        async fn open_download(
            &self,
            _hash: String,
            _offset: u64,
        ) -> Result<DummyReader, BlobError> {
            unimplemented!()
        }
        async fn delete_blob(&self, _hash: String) -> Result<(), BlobError> {
            unimplemented!()
        }
        async fn signed_url(&self, _hash: String, _ttl: u32) -> Result<String, BlobError> {
            unimplemented!()
        }
    }

    impl AppMessaging for TestHost {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), MessagingError> {
            unimplemented!()
        }
        async fn subscribe(&self, _topic: String) -> Result<(), MessagingError> {
            unimplemented!()
        }
        async fn unsubscribe(&self, _topic: String) -> Result<(), MessagingError> {
            unimplemented!()
        }
    }

    impl AppConversation for TestHost {
        async fn open_direct(&self, _peer: String) -> Result<String, ConversationError> {
            unimplemented!()
        }
        async fn conversations(&self) -> Result<Vec<ConversationSummary>, ConversationError> {
            unimplemented!()
        }
        async fn send(
            &self,
            _cid: String,
            _ctype: String,
            _body: Vec<u8>,
        ) -> Result<String, ConversationError> {
            unimplemented!()
        }
        async fn history(
            &self,
            _cid: String,
            _limit: u32,
            _cursor: Option<String>,
        ) -> Result<HistoryPage, ConversationError> {
            unimplemented!()
        }
        async fn delivery_status(&self, _mid: String) -> Result<DeliveryState, ConversationError> {
            unimplemented!()
        }
        async fn outbox(&self) -> Result<Vec<Message>, ConversationError> {
            unimplemented!()
        }
        async fn retry(&self, _mid: String) -> Result<(), ConversationError> {
            unimplemented!()
        }
        async fn create_group(&self) -> Result<String, ConversationError> {
            unimplemented!()
        }
        async fn add_member(&self, _cid: String, _member: String) -> Result<(), ConversationError> {
            unimplemented!()
        }
        async fn remove_member(
            &self,
            _cid: String,
            _member: String,
        ) -> Result<(), ConversationError> {
            unimplemented!()
        }
        async fn members(&self, _cid: String) -> Result<Vec<String>, ConversationError> {
            unimplemented!()
        }
        async fn membership_history(
            &self,
            _cid: String,
        ) -> Result<Vec<MembershipEvent>, ConversationError> {
            unimplemented!()
        }
        async fn sync_now(&self, _cid: String) -> Result<(), ConversationError> {
            unimplemented!()
        }
    }

    impl AppProxy for TestHost {
        async fn call(
            &self,
            _target: CallTarget,
            _iface: String,
            _method: String,
            _params: String,
            _opts: Option<CallOptions>,
        ) -> Result<String, ProxyError> {
            unimplemented!()
        }
        async fn enqueue(
            &self,
            _target: CallTarget,
            _iface: String,
            _method: String,
            _params: String,
            _opts: Option<CallOptions>,
        ) -> Result<(), ProxyError> {
            unimplemented!()
        }
    }

    impl AppAppConfig for TestHost {
        async fn get(&self, _key: String) -> Result<Option<String>, ConfigError> {
            unimplemented!()
        }
        async fn get_section(&self, _prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
            unimplemented!()
        }
    }

    impl AppVault for TestHost {
        async fn reveal(&self, _key: String) -> Result<Vec<u8>, VaultError> {
            unimplemented!()
        }
    }

    impl AppSigning for TestHost {
        async fn signing_identity(&self) -> Result<SigningIdentity, SigningError> {
            let id = self.signing_id.lock().unwrap();
            id.clone().ok_or_else(|| SigningError::Internal("no signing identity set".to_string()))
        }
        async fn sign_record(
            &self,
            _draft: RecordDraft,
            _principal: Principal,
        ) -> Result<String, SigningError> {
            unimplemented!()
        }
    }

    impl AppWebSocket for TestHost {
        async fn send(
            &self,
            _conn: String,
            _frame: Vec<u8>,
            _kind: FrameKind,
        ) -> Result<(), String> {
            unimplemented!()
        }
    }

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
            other => panic!("expected Installed, got {:?}", other),
        }

        // 5. Test Expired status
        match status(&host, cert.expires_at_secs + 1).await.unwrap() {
            CertificateStatus::Expired { expires_at_secs } => {
                assert_eq!(expires_at_secs, cert.expires_at_secs);
            }
            other => panic!("expected Expired, got {:?}", other),
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
            other => panic!("expected Stale, got {:?}", other),
        }
    }
}
