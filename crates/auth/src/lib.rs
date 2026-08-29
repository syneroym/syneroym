pub mod service;
pub mod token;

pub use service::{
    AuthService, ChallengeResponse, DEFAULT_NONCE_TTL_SECS, DEFAULT_SESSION_TTL_SECS,
    DelegatedKeyLoginParams, LocalLoginParams, LoginRequest, LoginResponse, MethodsResponse,
    WhoamiResponse,
};
pub use token::{
    AUTH_METHOD_DELEGATED_KEY, AUTH_METHOD_FIXED, AUTH_METHOD_LOCAL, CLAIM_AUTH_METHOD,
    SessionToken, SessionTokenClaims,
};

/// Verify a session token string against an auth service DID.
pub fn verify_session_token(
    token_str: &str,
    expected_auth_did: &str,
) -> anyhow::Result<SessionTokenClaims> {
    SessionToken::verify(token_str, expected_auth_did)
}
