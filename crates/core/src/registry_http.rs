//! Settings shared by every HTTP call made to a community registry.

use std::time;

/// How long a registry request waits before giving up. `reqwest::Client`
/// sets no default -- an unresponsive-but-not-refusing registry (packets
/// dropped rather than an RST) would otherwise stall the caller for the OS
/// connect timeout, which on some networks is minutes, not seconds. Every
/// call site treats a registry failure as non-fatal already (warn and
/// continue, or fall back to the DHT), so a short timeout only trades a
/// slightly less patient retry loop for never blocking the caller this long.
pub const HTTP_REQUEST_TIMEOUT: time::Duration = time::Duration::from_secs(10);
