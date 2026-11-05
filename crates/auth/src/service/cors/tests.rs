use super::matches_origin_pattern;

#[test]
fn a_subdomain_pattern_allows_every_label_of_its_domain_and_nothing_else() {
    let pattern = "http://*.roym.test:8443";
    for allowed in ["http://a.roym.test:8443", "http://a-b.c.roym.test:8443"] {
        assert!(matches_origin_pattern(pattern, allowed), "{allowed} must match");
    }
    for refused in [
        "http://roym.test:8443",
        "http://evilroym.test:8443",
        "https://a.roym.test:8443",
        "http://a.roym.test:8444",
        "http://a.roym.test",
        "http://.roym.test:8443",
        "http://a..roym.test:8443",
        "http://user@a.roym.test:8443",
    ] {
        assert!(!matches_origin_pattern(pattern, refused), "{refused} must not match");
    }
}

#[test]
fn a_pattern_with_no_port_matches_only_origins_with_no_port() {
    assert!(matches_origin_pattern("https://*.example.com", "https://a.example.com"));
    assert!(!matches_origin_pattern("https://*.example.com", "https://a.example.com:8443"));
}

#[test]
fn an_entry_that_is_not_a_subdomain_pattern_never_matches() {
    assert!(!matches_origin_pattern("https://example.com", "https://a.example.com"));
    assert!(!matches_origin_pattern("https://*.", "https://a."));
    assert!(!matches_origin_pattern("*.example.com", "https://a.example.com"));
}
