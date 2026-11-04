use super::AuthService;

impl AuthService {
    fn is_allowed_origin(&self, origin: &str) -> bool {
        let trimmed = origin.trim();
        if self.allowed_origins.iter().any(|allowed| allowed == trimmed) {
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
