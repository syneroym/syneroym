use std::collections::HashSet;

use super::*;

#[test]
fn every_code_is_distinct() {
    let all = [
        RegisterError::NotAccepted,
        RegisterError::RateLimited,
        RegisterError::RequestTooLarge { max: 1 },
        RegisterError::Malformed(String::new()),
        RegisterError::UnknownChallenge,
        RegisterError::ChallengeExpired,
        RegisterError::BadSignature,
        RegisterError::NotBound,
        RegisterError::UntrustedRoot,
        RegisterError::BrokenChain,
        RegisterError::WrongCapability { ability: String::new(), resource: String::new() },
        RegisterError::Expired,
        RegisterError::InvalidChain(String::new()),
        RegisterError::Denied,
        RegisterError::Revoked,
        RegisterError::LimitReached { limit: 1 },
        RegisterError::Store(String::new()),
    ];
    let codes: HashSet<&str> = all.iter().map(RegisterError::code).collect();
    assert_eq!(codes.len(), all.len());
}
