use std::fmt::{self, Display, Formatter};

use super::RoutePreamble;

/// A log view of a preamble: no signature, certificate, capability chain
/// or `on_behalf` value.
pub struct RedactedPreamble<'a>(pub(crate) &'a RoutePreamble);

impl fmt::Debug for RedactedPreamble<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for RedactedPreamble<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let base = self.0.route_base();
        let mut params = Vec::new();
        if let Some(enc) = &self.0.enc {
            params.push(format!("enc={enc}"));
        }
        if let Some(dir) = &self.0.dir {
            params.push(format!("dir={dir}"));
        }
        if let Some(pubkey) = &self.0.pubkey {
            params.push(format!("pubkey={pubkey}"));
        }
        let master = self.0.delegation.as_ref().map(|c| c.master_did.as_str()).unwrap_or("-");
        params.push(format!("master={master}"));
        if let Some(ts) = self.0.ts {
            params.push(format!("ts={ts}"));
        }
        let epk = if self.0.epk.is_some() { "set" } else { "-" };
        params.push(format!("epk={epk}"));
        let sig = if self.0.sig.is_some() { "set" } else { "-" };
        params.push(format!("sig={sig}"));
        let ucan = if self.0.ucan.is_some() { "set" } else { "-" };
        params.push(format!("ucan={ucan}"));
        let on_behalf = if self.0.on_behalf.is_some() { "set" } else { "-" };
        params.push(format!("on_behalf={on_behalf}"));

        write!(f, "{base}?{}", params.join("&"))
    }
}

/// The route part of a raw preamble line (text before `?`), at most 256
/// characters, for error messages and logs.
#[must_use]
pub fn route_text(raw: &str) -> &str {
    let trimmed = raw.trim();
    let before_q = trimmed.split_once('?').map_or(trimmed, |(b, _)| b);
    if before_q.len() > 256 {
        let mut end = 256;
        while !before_q.is_char_boundary(end) {
            end -= 1;
        }
        &before_q[..end]
    } else {
        before_q
    }
}
