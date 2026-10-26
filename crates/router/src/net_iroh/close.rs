//! Closing an Iroh endpoint at shutdown without waiting for the slowest peer.

use std::{future::Future, time::Duration};

use tokio::{task, time};
use tracing::{error, warn};

/// The longest a shutdown waits for the connections of an Iroh endpoint to
/// finish closing. Iroh documents 3 seconds for a close with default settings
/// and a conservative round-trip estimate, when connectivity is bad.
///
/// `Endpoint::close` returns only when every connection has ended its closing
/// period. That period is three times the largest probe timeout of any path of
/// the connection, and the probe timeout grows with the round-trip estimate of
/// the path. A path with a bad estimate (a relay path of a busy host, a mobile
/// link) makes the wait tens of seconds: we measured 14 s and 39 s on one host
/// under load. The peer is told about the close when the call begins, so the
/// wait only serves to repeat that message if it was lost. A node that is
/// shutting down must not depend on it.
pub const ENDPOINT_CLOSE_LIMIT: Duration = Duration::from_secs(3);

/// Runs `close`, which closes one or more Iroh endpoints, and waits at most
/// `limit` for it. `what` names the endpoint in the log.
///
/// Returns whether `close` finished in time. When it did not, it keeps running
/// in the background until it is done, so the endpoint is not left half
/// closed: dropping a `close` future part way would leave the sockets and the
/// tasks of the endpoint open for the rest of the process.
pub async fn close_within<F>(what: &str, limit: Duration, close: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut closing = task::spawn(close);
    match time::timeout(limit, &mut closing).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            error!(what, error = %e, "closing the endpoint failed");
            true
        }
        Err(_) => {
            warn!(
                what,
                limit_secs = limit.as_secs_f32(),
                "the peers have not confirmed the close in time; it finishes in the background"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests;
