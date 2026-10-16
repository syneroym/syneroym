//! The passport side of a relay with passport access: the lease store, the
//! two registration endpoints on the info server, and the DID of a relay
//! client.

use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        ConnectInfo, DefaultBodyLimit, State,
        rejection::{BytesRejection, FailedToBufferBody},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing,
};
use ed25519_dalek::VerifyingKey;
use iroh::EndpointId;
use syneroym_core::{dht_registry::MasterAnchorResolver, host_identity::HostIdentity};
use syneroym_identity::substrate;
use syneroym_passport::{
    CHALLENGE_LIFE_SECS, Lease, LeaseStore, RELAY_HOME, RegisterError, Registrar, RequestLimiter,
    SignerBinding,
    wire::{self, ChallengeResponse, ErrorBody, RegisterRequest, RegisterResponse},
};
use syneroym_router::ucan_chain_not_revoked;
use tokio::{task, time};
use tracing::{debug, error, info, warn};

/// The lease file of the relay, in `storage.db_dir`.
pub(crate) const RELAY_LEASES_DB: &str = "relay_passports.db";

/// Maximum time allowed for revocation lookup across chain edges.
const REVOCATION_LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

/// What the registration endpoints of one relay need.
#[derive(Debug)]
pub(crate) struct RelayPassports {
    registrar: Arc<Registrar>,
    limiter: RequestLimiter,
    resolver: Arc<dyn MasterAnchorResolver>,
}

impl RelayPassports {
    /// Open the lease store in `db_dir` and the registrar for the home-relay
    /// use, rooted at the host's passport root (ADR-0015 A10 point 4).
    pub(crate) async fn open(
        db_dir: &Path,
        host: &HostIdentity,
        resolver: Arc<dyn MasterAnchorResolver>,
    ) -> Result<Self> {
        let dir = db_dir.to_path_buf();
        let leases = task::spawn_blocking(move || {
            fs::create_dir_all(&dir)?;
            LeaseStore::open(&dir, RELAY_LEASES_DB, &RELAY_HOME, syneroym_passport::now_secs())
        })
        .await
        .context("the relay lease store task failed")?
        .context("could not open the relay lease store")?;
        let root = host.passport_root().to_string();
        let registrar =
            Registrar::new(RELAY_HOME, host.node_did.clone(), vec![root.clone()], leases)
                .context("[roles.coordinator] access accepts passports")?;
        info!(%root, "the relay accepts passports issued by this DID");
        Ok(Self { registrar: Arc::new(registrar), limiter: RequestLimiter::default(), resolver })
    }

    pub(crate) fn leases(&self) -> &LeaseStore {
        self.registrar.leases()
    }

    fn challenge(&self, peer: IpAddr, now: u64) -> Result<ChallengeResponse, RegisterError> {
        if !self.limiter.allow(peer, now) {
            return Err(RegisterError::RateLimited);
        }
        let challenge = self
            .registrar
            .issue_challenge(now)
            .map_err(|e| RegisterError::Store(format!("{e:#}")))?;
        Ok(ChallengeResponse {
            challenge: challenge.to_hex(),
            expires_in_secs: CHALLENGE_LIFE_SECS,
        })
    }

    /// Limit, size and shape, the offline checks, the DHT revocation check
    /// on every chain edge, then the lease write.
    async fn register(
        &self,
        peer: IpAddr,
        body: Result<Bytes, BytesRejection>,
        now: u64,
    ) -> Result<Lease, RegisterError> {
        if !self.limiter.allow(peer, now) {
            return Err(RegisterError::RateLimited);
        }
        let body = body.map_err(body_error)?;
        let (token, proof) = RegisterRequest::from_slice(&body)?.into_parts()?;
        let checked = self.registrar.check(&token, &proof, &SignerBinding::SameKey, now)?;
        // ADR-0015 §2 and A7: a revoked edge stops the registration. The
        // lookup needs the network, so it runs only after every offline
        // check passed. Bounded by a timeout; on timeout or network
        // failure it is treated as not revoked.
        let revocation_check = ucan_chain_not_revoked(checked.token(), self.resolver.as_ref());
        match time::timeout(REVOCATION_LOOKUP_TIMEOUT, revocation_check).await {
            Ok(false) => return Err(RegisterError::Revoked),
            Ok(true) => {}
            Err(_) => {
                warn!("revocation lookup timed out; treating as not revoked");
            }
        }
        let registrar = Arc::clone(&self.registrar);
        // A fresh clock: the lookup above may take seconds.
        let lease =
            task::spawn_blocking(move || registrar.grant(checked, syneroym_passport::now_secs()))
                .await
                .map_err(|e| RegisterError::Store(format!("the lease write task failed: {e}")))??;
        info!(passport_id = %lease.passport_id, subject = %lease.subject,
              lease_ends_at = lease.lease_ends_at, "relay passport lease granted");
        Ok(lease)
    }
}

fn body_error(e: BytesRejection) -> RegisterError {
    match e {
        BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
            RegisterError::RequestTooLarge { max: wire::MAX_REQUEST_BYTES }
        }
        other => RegisterError::Malformed(format!("could not read the request body: {other}")),
    }
}

type PassportState = Option<Arc<RelayPassports>>;

/// `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.
pub(crate) fn routes(passports: PassportState) -> Router {
    Router::new()
        .route(wire::CHALLENGE_PATH, routing::post(post_challenge))
        .route(wire::REGISTER_PATH, routing::post(post_register))
        .layer(DefaultBodyLimit::max(wire::MAX_REQUEST_BYTES))
        .with_state(passports)
}

async fn post_challenge(
    State(passports): State<PassportState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    let Some(passports) = passports else { return refusal(&RegisterError::NotAccepted) };
    respond(passports.challenge(peer.ip(), syneroym_passport::now_secs()))
}

async fn post_register(
    State(passports): State<PassportState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(passports) = passports else { return refusal(&RegisterError::NotAccepted) };
    let result = passports.register(peer.ip(), body, syneroym_passport::now_secs()).await;
    respond(result.map(|lease| RegisterResponse {
        passport_id: lease.passport_id,
        lease_ends_at: lease.lease_ends_at,
    }))
}

fn respond<T: serde::Serialize>(result: Result<T, RegisterError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(e) => refusal(&e),
    }
}

/// The error body of `e`. A store error is logged here with its full text,
/// because the body carries only "internal error".
fn refusal(e: &RegisterError) -> Response {
    if let RegisterError::Store(_) = e {
        error!(error = %e, "relay passport store error");
    } else {
        debug!(code = e.code(), "relay passport registration refused: {e}");
    }
    (status_of(e), Json(ErrorBody::from(e))).into_response()
}

fn status_of(e: &RegisterError) -> StatusCode {
    match e {
        RegisterError::RequestTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        RegisterError::Malformed(_)
        | RegisterError::UnknownChallenge
        | RegisterError::ChallengeExpired => StatusCode::BAD_REQUEST,
        RegisterError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        RegisterError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        RegisterError::BadSignature
        | RegisterError::NotBound
        | RegisterError::UntrustedRoot
        | RegisterError::BrokenChain
        | RegisterError::WrongCapability { .. }
        | RegisterError::Expired
        | RegisterError::InvalidChain(_)
        | RegisterError::Denied
        | RegisterError::Revoked
        | RegisterError::LimitReached { .. }
        | RegisterError::NotAccepted => StatusCode::FORBIDDEN,
    }
}

/// The DID of a relay client: its endpoint id is its ed25519 key. `None` for
/// bytes that are not a valid key (the relay then refuses it).
pub(crate) fn endpoint_did(id: &EndpointId) -> Option<String> {
    VerifyingKey::from_bytes(id.as_bytes()).ok().map(|key| substrate::derive_did_key(&key))
}

/// The endpoint id of a `did:key` (the host node's Iroh endpoint).
pub(crate) fn endpoint_of_did(did: &str) -> Result<EndpointId> {
    let key = substrate::resolve_did_key(did)?;
    EndpointId::from_bytes(key.as_bytes()).with_context(|| format!("{did} is not an Iroh key"))
}

/// Whether the client `id` has a running lease.
pub(crate) fn is_leased(leases: &LeaseStore, id: &EndpointId) -> bool {
    endpoint_did(id).is_some_and(|did| leases.is_admitted(&did, syneroym_passport::now_secs()))
}

#[cfg(test)]
mod tests;
