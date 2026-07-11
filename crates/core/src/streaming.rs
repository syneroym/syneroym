//! Shared types for M3B Slice 6B bidirectional streaming (ADR-0014).
//!
//! Lives in `syneroym-core` (not `syneroym-router` or `syneroym-sandbox-wasm`)
//! because both crates need it and `router` already depends on
//! `sandbox-wasm` -- a shared dependency avoids a cycle.

use std::{fmt, str::FromStr};

/// Which side pulls vs. pushes on a `raw://<protocol>|<service_id>?dir=...`
/// stream-protocol request. Parsed from the `dir` query parameter on
/// `RoutePreamble` (`crates/router/src/preamble.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDirection {
    /// Peer pulls from the guest: routes to `guest-api::handle-stream-request`
    /// / `stream-cursor`.
    Download,
    /// Peer pushes into the guest: routes to `guest-api::accept-stream-upload`
    /// / `stream-sink`.
    Upload,
}

impl fmt::Display for StreamDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Download => "download",
            Self::Upload => "upload",
        })
    }
}

impl FromStr for StreamDirection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "download" => Ok(Self::Download),
            "upload" => Ok(Self::Upload),
            other => Err(format!("invalid stream direction '{other}': expected upload|download")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_display_and_from_str() {
        assert_eq!(StreamDirection::Download.to_string(), "download");
        assert_eq!(StreamDirection::Upload.to_string(), "upload");
        assert_eq!("download".parse::<StreamDirection>().unwrap(), StreamDirection::Download);
        assert_eq!("upload".parse::<StreamDirection>().unwrap(), StreamDirection::Upload);
    }

    #[test]
    fn rejects_anything_else() {
        assert!("sideways".parse::<StreamDirection>().is_err());
        assert!("".parse::<StreamDirection>().is_err());
    }
}
