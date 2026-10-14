//! The refusals of a passport registration, each with a stable wire code.

use crate::challenge_store::CHALLENGE_LIFE_SECS;

/// Why a registration was refused. `code()` is the stable string a service
/// puts in its HTTP or RPC error.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegisterError {
    #[error("request body is larger than {max} bytes")]
    RequestTooLarge { max: usize },
    #[error("malformed request: {0}")]
    Malformed(String),
    #[error("unknown challenge: never issued, already used, or dropped")]
    UnknownChallenge,
    #[error("the challenge is older than {CHALLENGE_LIFE_SECS} seconds")]
    ChallengeExpired,
    #[error("the challenge signature does not verify")]
    BadSignature,
    #[error("the proven key may not act for the subject")]
    NotBound,
    #[error("the passport is not issued by an accepted operator root")]
    UntrustedRoot,
    #[error("the child token is not issued by the passport audience")]
    BrokenChain,
    #[error("the token does not name ability {ability} on {resource} exactly")]
    WrongCapability { ability: String, resource: String },
    #[error("the passport or the child token has expired")]
    Expired,
    #[error("the token chain is not valid: {0}")]
    InvalidChain(String),
    #[error("the passport is denied")]
    Denied,
    /// Made by the service, not by this crate: the DHT revocation check
    /// needs the network, so it runs between `Registrar::check` and
    /// `Registrar::grant`. Defined here so all services share one error set
    /// and one set of wire codes.
    #[error("a key in the token chain is revoked by its issuer")]
    Revoked,
    #[error("the passport admits at most {limit} subjects")]
    LimitReached { limit: u64 },
    #[error("lease store error: {0}")]
    Store(String),
}

impl RegisterError {
    /// The stable wire code of this refusal.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RequestTooLarge { .. } => "request_too_large",
            Self::Malformed(_) => "malformed",
            Self::UnknownChallenge => "unknown_challenge",
            Self::ChallengeExpired => "challenge_expired",
            Self::BadSignature => "bad_signature",
            Self::NotBound => "not_bound",
            Self::UntrustedRoot => "untrusted_root",
            Self::BrokenChain => "broken_chain",
            Self::WrongCapability { .. } => "wrong_capability",
            Self::Expired => "expired",
            Self::InvalidChain(_) => "invalid_chain",
            Self::Denied => "denied",
            Self::Revoked => "revoked",
            Self::LimitReached { .. } => "limit_reached",
            Self::Store(_) => "internal",
        }
    }
}

#[cfg(test)]
mod tests;
