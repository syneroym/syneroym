//! One admission rule for every service behind the entrypoint.
//!
//! The person's browser reaches this app through one service, which checks
//! its own session before it forwards anything. No other ingress is
//! legitimate, and the address a person publishes so others can message
//! them is the same string that addresses this service's API -- so
//! "reachable" and "meant for you" are different questions, and only the
//! host can answer the first.

use syneroym_app_host::{AppHost, AppInvocation, types::invocation::CallerOrigin};

use crate::envelope::Response;

/// Returned to a caller that did not arrive through a local dispatch path.
pub const NOT_LOCAL: i64 = -32013;

/// `None` admits. `Some(response)` is the refusal to return unchanged.
/// Deliberately not a `bool`: a refusal that a caller has to remember to
/// turn into a response is a refusal somebody eventually forgets.
pub async fn require_internal<H: AppHost>(host: &H) -> Option<Response> {
    match AppInvocation::caller(host).await {
        CallerOrigin::Internal => None,
        // The refusal names no DID and no service: a stranger learns only
        // that this method is not theirs to call.
        CallerOrigin::Verified(_) | CallerOrigin::Anonymous => Some(Response::err(
            NOT_LOCAL,
            "this method is reachable only from inside this installation",
        )),
    }
}

/// What a method allows from off this node. Absent from a service's table
/// means `LocalOnly`: an exception is written down or it does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRule {
    /// Any caller, identified or not. Reading something this installation
    /// publishes on purpose.
    Open,
    /// A caller whose identity the router verified. The DID names the
    /// calling service's node, never a person -- a record's issuer is the
    /// only thing that names a person.
    VerifiedOnly,
}

/// Who is on the other end, once admission has already been decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Internal,
    Verified(String),
    Anonymous,
}

/// `Ok(caller)` admits. `Err(response)` is the refusal to return unchanged,
/// with the same code and the same wording `require_internal` uses -- a
/// stranger learns only that this method is not theirs to call.
pub async fn admit<H: AppHost>(
    host: &H,
    exceptions: &[(&str, WireRule)],
    method: &str,
) -> Result<Caller, Response> {
    match AppInvocation::caller(host).await {
        CallerOrigin::Internal => Ok(Caller::Internal),
        origin => {
            let refused = || {
                Err(Response::err(
                    NOT_LOCAL,
                    "this method is reachable only from inside this installation",
                ))
            };
            let Some((_, rule)) = exceptions.iter().find(|(m, _)| *m == method) else {
                return refused();
            };
            match (rule, origin) {
                (WireRule::Open, CallerOrigin::Verified(did)) => Ok(Caller::Verified(did)),
                (WireRule::Open, CallerOrigin::Anonymous) => Ok(Caller::Anonymous),
                (WireRule::VerifiedOnly, CallerOrigin::Verified(did)) => Ok(Caller::Verified(did)),
                (WireRule::VerifiedOnly, CallerOrigin::Anonymous) => refused(),
                (_, CallerOrigin::Internal) => unreachable!("handled above"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use syneroym_app_host::types::invocation::CallerOrigin;

    use super::*;
    use crate::signing::tests::TestHost;

    #[tokio::test]
    async fn internal_is_admitted_and_the_two_wire_arms_are_refused() {
        let host = TestHost::default(); // defaults to Internal
        assert!(require_internal(&host).await.is_none());

        host.set_caller_origin(CallerOrigin::Verified("did:key:zStranger".to_string()));
        let refused = require_internal(&host).await.expect("verified stranger is refused");
        assert_eq!(refused.error.as_ref().unwrap().code, NOT_LOCAL);
        // Neither the DID nor a service name leaks into the message.
        assert!(!refused.error.as_ref().unwrap().message.contains("did:key"));

        host.set_caller_origin(CallerOrigin::Anonymous);
        assert_eq!(require_internal(&host).await.unwrap().error.unwrap().code, NOT_LOCAL);
    }

    const TABLE: &[(&str, WireRule)] =
        &[("directory.search", WireRule::Open), ("directory.publish", WireRule::VerifiedOnly)];

    #[tokio::test]
    async fn internal_admits_for_a_method_absent_from_the_table() {
        let host = TestHost::default();
        assert_eq!(admit(&host, TABLE, "directory.unpublish").await, Ok(Caller::Internal));
    }

    #[tokio::test]
    async fn an_absent_method_refuses_for_both_wire_arms() {
        let host = TestHost::default();
        host.set_caller_origin(CallerOrigin::Verified("did:key:zStranger".to_string()));
        assert_eq!(
            admit(&host, TABLE, "directory.unpublish").await.unwrap_err().error.unwrap().code,
            NOT_LOCAL
        );
        host.set_caller_origin(CallerOrigin::Anonymous);
        assert_eq!(
            admit(&host, TABLE, "directory.unpublish").await.unwrap_err().error.unwrap().code,
            NOT_LOCAL
        );
    }

    #[tokio::test]
    async fn an_open_method_admits_both_wire_arms_and_returns_the_right_caller() {
        let host = TestHost::default();
        host.set_caller_origin(CallerOrigin::Verified("did:key:zStranger".to_string()));
        assert_eq!(
            admit(&host, TABLE, "directory.search").await,
            Ok(Caller::Verified("did:key:zStranger".to_string()))
        );
        host.set_caller_origin(CallerOrigin::Anonymous);
        assert_eq!(admit(&host, TABLE, "directory.search").await, Ok(Caller::Anonymous));
    }

    #[tokio::test]
    async fn a_verified_only_method_admits_verified_and_refuses_anonymous() {
        let host = TestHost::default();
        host.set_caller_origin(CallerOrigin::Verified("did:key:zStranger".to_string()));
        assert_eq!(
            admit(&host, TABLE, "directory.publish").await,
            Ok(Caller::Verified("did:key:zStranger".to_string()))
        );
        host.set_caller_origin(CallerOrigin::Anonymous);
        assert_eq!(
            admit(&host, TABLE, "directory.publish").await.unwrap_err().error.unwrap().code,
            NOT_LOCAL
        );
    }

    #[tokio::test]
    async fn anonymous_refusal_on_verified_only_matches_unlisted_refusal_byte_for_byte() {
        let host = TestHost::default();
        host.set_caller_origin(CallerOrigin::Anonymous);
        let a = admit(&host, TABLE, "directory.publish").await.unwrap_err();
        let b = admit(&host, TABLE, "directory.unpublish").await.unwrap_err();
        assert_eq!(a, b);
    }
}
