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
}
