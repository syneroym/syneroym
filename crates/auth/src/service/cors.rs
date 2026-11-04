use super::AuthService;

impl AuthService {
    fn is_allowed_origin(&self, origin: &str) -> bool {
        let trimmed = origin.trim();
        if self
            .allowed_origins
            .iter()
            .any(|allowed| allowed == trimmed || matches_origin_pattern(allowed, trimmed))
        {
            return true;
        }
        trimmed.starts_with("http://localhost:")
            || trimmed.starts_with("https://localhost:")
            || trimmed == "http://localhost"
            || trimmed == "https://localhost"
            || trimmed.starts_with("http://127.0.0.1:")
            || trimmed.starts_with("https://127.0.0.1:")
            || trimmed == "http://127.0.0.1"
            || trimmed == "https://127.0.0.1"
            || (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
                && (trimmed.contains(".localhost:") || trimmed.ends_with(".localhost"))
    }

    pub(super) fn cors_headers(&self, origin: Option<&str>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("access-control-allow-methods".to_string(), "GET, POST, OPTIONS".to_string()),
            (
                "access-control-allow-headers".to_string(),
                "content-type, authorization, x-syneroym-routing-key".to_string(),
            ),
        ];
        if let Some(orig) = origin
            && self.is_allowed_origin(orig)
        {
            headers.push(("access-control-allow-origin".to_string(), orig.to_string()));
            headers.push(("access-control-allow-credentials".to_string(), "true".to_string()));
            headers.push(("vary".to_string(), "Origin".to_string()));
        }
        headers
    }
}

/// `https://*.example.com:8443` allows `https://<one or more labels>.example.com:8443`.
///
/// The scheme and the port (or the lack of one) must be equal. The labels
/// before the domain may hold only letters, digits and `-`, so the match
/// cannot be fooled by a host such as `evilexample.com` or one that smuggles
/// in a path or a userinfo part.
fn matches_origin_pattern(pattern: &str, origin: &str) -> bool {
    let (Some((pattern_scheme, pattern_rest)), Some((origin_scheme, origin_rest))) =
        (pattern.split_once("://"), origin.split_once("://"))
    else {
        return false;
    };
    let Some(suffix) = pattern_rest.strip_prefix("*.").filter(|s| !s.is_empty()) else {
        return false;
    };
    if pattern_scheme != origin_scheme {
        return false;
    }
    let Some(labels) = origin_rest.strip_suffix(suffix).and_then(|l| l.strip_suffix('.')) else {
        return false;
    };
    !labels.is_empty()
        && labels
            .split('.')
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

#[cfg(test)]
mod tests;
