//! Retry helper for `crates/substrate/tests/*.rs` integration suites, kept
//! separate from `common/mod.rs`'s `SubstrateTestContext` bootstrap so a
//! consuming file that only needs the macro below doesn't have to pull in
//! (and trip dead-code warnings on) that unrelated struct.

/// Runs a `SyneroymClient` call expression, retrying it once after an
/// explicit reconnect if it fails.
///
/// A client's connection is dialed and proven live during boot, then can
/// sit idle through whatever setup runs before a test's first real call on
/// it -- long enough under CI's scheduling pressure for the peer to
/// abandon that idle path ("no viable network path exists: last path
/// abandoned by peer"). Retrying the same request on that dead connection
/// does not help, since `connect` no-ops when the connection is already
/// `Some`, so recovery needs an explicit `shutdown`-then-`connect` (redial)
/// before the one retry.
///
/// A macro rather than a generic function: the retried call sites carry
/// arguments borrowed from the enclosing test (e.g. `&service_id`), and a
/// closure-based helper would need every caller to clone those into an
/// owned future just to satisfy the higher-ranked lifetime bound a generic
/// version would require -- more ceremony at every call site than the
/// duplication this replaces. `$client` is evaluated once; `$call` is a
/// full call expression on it (e.g.
/// `node.substrate_client.instance_identity(&service_id).await`) and is
/// evaluated again verbatim on retry.
///
/// The four-argument form is shorthand for the common
/// `$client.request($interface, $method, $params).await` shape (most call
/// sites); `$params` is cloned for the first attempt so the caller keeps
/// ownership for the retry. Calls through any other method (`lookup`,
/// `status`, `inject_kek`, ...) use the two-argument form directly.
#[macro_export]
macro_rules! call_with_reconnect {
    ($client:expr, $interface:expr, $method:expr, $params:expr) => {
        $crate::call_with_reconnect!(
            $client,
            $client.request($interface, $method, $params.clone()).await
        )
    };
    ($client:expr, $call:expr) => {
        match $call {
            Ok(value) => value,
            Err(_) => {
                $client.shutdown().await.expect("failed to reset stale connection");
                $client.connect().await.expect("failed to reconnect");
                $call.expect("request failed even after reconnect")
            }
        }
    };
}
