//! HTTP registration client for home relay passports.

use std::{future::Future, sync::Mutex, time::Duration};

use reqwest::redirect;
use serde::de::DeserializeOwned;
use syneroym_identity::Identity;
use syneroym_passport::{
    CapabilityToken, Challenge, RELAY_HOME, sign_challenge,
    wire::{
        CHALLENGE_PATH, ChallengeResponse, ErrorBody, REGISTER_PATH, RegisterRequest,
        RegisterResponse, RetryClass, UNREACHABLE_CODE,
    },
};
use syneroym_router::HomeRelay;

const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(10);
/// Above the coordinator's 15 s revocation lookup, so the client does not
/// give up while the coordinator still works on the request.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(30);
/// The most bytes of an answer that are read.
const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// The largest accepted `ca_cert_pem`.
pub(super) const MAX_CA_CERT_BYTES: usize = 64 * 1024;

/// Where and how to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) info_url: String,
    pub(super) ca_cert_pem: Option<String>,
}

/// Why an attempt did not give a lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Failure {
    /// The coordinator answered with a refusal body.
    Refused(ErrorBody),
    /// No answer, a timeout, or an answer that is not a refusal body.
    Unreachable(String),
}

impl Failure {
    pub(super) fn body(&self) -> ErrorBody {
        match self {
            Self::Refused(body) => body.clone(),
            Self::Unreachable(msg) => {
                ErrorBody { code: UNREACHABLE_CODE.to_owned(), message: msg.clone() }
            }
        }
    }

    pub(super) fn class(&self) -> RetryClass {
        match self {
            Self::Refused(body) => RetryClass::of(&body.code),
            Self::Unreachable(_) => RetryClass::Backoff,
        }
    }
}

/// The network half of the agent. Unit tests replace it with a fake and a
/// paused clock.
pub(super) trait RegistrationIo: Send + Sync + 'static {
    fn challenge(
        &self,
        target: &Target,
    ) -> impl Future<Output = Result<ChallengeResponse, Failure>> + Send;
    fn register(
        &self,
        target: &Target,
        request: &RegisterRequest,
    ) -> impl Future<Output = Result<RegisterResponse, Failure>> + Send;
    /// `HomeRelay::closed`.
    fn closed(&self) -> impl Future<Output = ()> + Send;
    /// Unix seconds now.
    fn now_secs(&self) -> u64;
}

pub(super) struct LiveIo {
    home_relay: HomeRelay,
    /// The client of the last target, rebuilt when its certificate changes.
    client: Mutex<Option<(Option<String>, reqwest::Client)>>,
}

impl LiveIo {
    pub(super) fn new(home_relay: HomeRelay) -> Self {
        Self { home_relay, client: Mutex::new(None) }
    }

    fn get_client(&self, ca_cert_pem: Option<&str>) -> Result<reqwest::Client, Failure> {
        let mut guard = match self.client.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some((cached_pem, client)) = &*guard
            && cached_pem.as_deref() == ca_cert_pem
        {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .pool_max_idle_per_host(0)
            .redirect(redirect::Policy::none())
            .timeout(REGISTER_TIMEOUT);
        if let Some(pem) = ca_cert_pem {
            let cert = reqwest::Certificate::from_pem(pem.as_bytes())
                .map_err(|e| Failure::Unreachable(format!("invalid root certificate: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }
        let client = builder
            .build()
            .map_err(|e| Failure::Unreachable(format!("cannot build HTTP client: {e}")))?;
        *guard = Some((ca_cert_pem.map(ToOwned::to_owned), client.clone()));
        Ok(client)
    }
}

pub(super) async fn parse_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, Failure> {
    let status = response.status();
    let body = super::super::read_capped(response, MAX_ANSWER_BYTES)
        .await
        .map_err(|e| Failure::Unreachable(e.to_string()))?;
    if status.is_success() {
        serde_json::from_slice::<T>(&body)
            .map_err(|e| Failure::Unreachable(format!("the answer is not valid: {e}")))
    } else {
        match serde_json::from_slice::<ErrorBody>(&body) {
            Ok(refused) => Err(Failure::Refused(refused)),
            Err(_) => Err(Failure::Unreachable(format!("HTTP {status}"))),
        }
    }
}

impl RegistrationIo for LiveIo {
    async fn challenge(&self, target: &Target) -> Result<ChallengeResponse, Failure> {
        let client = self.get_client(target.ca_cert_pem.as_deref())?;
        let url = format!("{}{CHALLENGE_PATH}", target.info_url);
        let response = client
            .post(&url)
            .timeout(CHALLENGE_TIMEOUT)
            .send()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        parse_response(response).await
    }

    async fn register(
        &self,
        target: &Target,
        request: &RegisterRequest,
    ) -> Result<RegisterResponse, Failure> {
        let client = self.get_client(target.ca_cert_pem.as_deref())?;
        let url = format!("{}{REGISTER_PATH}", target.info_url);
        let response = client
            .post(&url)
            .timeout(REGISTER_TIMEOUT)
            .json(request)
            .send()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        parse_response(response).await
    }

    async fn closed(&self) {
        self.home_relay.closed().await;
    }

    fn now_secs(&self) -> u64 {
        syneroym_passport::now_secs()
    }
}

/// Challenge, sign, register. Asks for one new challenge when the first
/// is unknown or too old (both codes mean the same to a client).
pub(super) async fn register_once<I: RegistrationIo>(
    io: &I,
    target: &Target,
    node: &Identity,
    node_did: &str,
    host_node_did: &str,
    token: &CapabilityToken,
) -> Result<RegisterResponse, Failure> {
    let mut new_challenge_left = true;
    loop {
        let answer = io.challenge(target).await?;
        let challenge = Challenge::from_hex(&answer.challenge)
            .map_err(|e| Failure::Unreachable(format!("the challenge is not valid: {e}")))?;
        // A signing error cannot really happen with a valid key; it is reported
        // as unreachable rather than adding a third failure kind.
        let signature = sign_challenge(node, host_node_did, &RELAY_HOME, &challenge, token)
            .map_err(|e| Failure::Unreachable(format!("cannot sign the challenge: {e:#}")))?;
        let request = RegisterRequest {
            token: token.clone(),
            challenge: answer.challenge,
            signer_did: node_did.to_owned(),
            signature,
        };
        match io.register(target, &request).await {
            Err(f) if f.class() == RetryClass::NewChallenge && new_challenge_left => {
                new_challenge_left = false;
            }
            other => return other,
        }
    }
}
