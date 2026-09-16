use super::*;

// ---- proxy: in (guest -> host) ----

pub(crate) fn call_target_in(v: GuestCallTarget) -> HostCallTarget {
    match v {
        GuestCallTarget::Service(s) => HostCallTarget::Service(s),
        GuestCallTarget::Dependency(s) => HostCallTarget::Dependency(s),
    }
}

pub(crate) fn call_options_in(v: GuestCallOptions) -> HostCallOptions {
    HostCallOptions {
        protocol: v.protocol,
        idempotent: v.idempotent,
        timeout_ms: v.timeout_ms,
        routing_key: v.routing_key,
        idempotency_key: v.idempotency_key,
    }
}

// ---- proxy: out (host -> guest) ----

pub(crate) fn callee_error_out(v: HostCalleeError) -> GuestCalleeError {
    GuestCalleeError { code: v.code, message: v.message, data: v.data }
}

pub(crate) fn proxy_error_out(v: HostProxyError) -> GuestProxyError {
    match v {
        HostProxyError::ServiceNotFound(s) => GuestProxyError::ServiceNotFound(s),
        HostProxyError::DependencyNotBound(s) => GuestProxyError::DependencyNotBound(s),
        HostProxyError::UnsupportedProtocol(s) => GuestProxyError::UnsupportedProtocol(s),
        HostProxyError::UnsupportedTarget(s) => GuestProxyError::UnsupportedTarget(s),
        HostProxyError::PermissionDenied(s) => GuestProxyError::PermissionDenied(s),
        HostProxyError::Transport(s) => GuestProxyError::Transport(s),
        HostProxyError::TimedOut => GuestProxyError::TimedOut,
        HostProxyError::Callee(e) => GuestProxyError::Callee(callee_error_out(e)),
        HostProxyError::Internal(s) => GuestProxyError::Internal(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_target_and_options_round_trip() {
        let t = call_target_in(GuestCallTarget::Service("svc1".to_string()));
        assert!(matches!(t, HostCallTarget::Service(s) if s == "svc1"));

        let t2 = call_target_in(GuestCallTarget::Dependency("dep1".to_string()));
        assert!(matches!(t2, HostCallTarget::Dependency(s) if s == "dep1"));

        let opt = call_options_in(GuestCallOptions {
            protocol: Some("json-rpc/v1".to_string()),
            idempotent: true,
            timeout_ms: Some(5000),
            routing_key: Some("rk".to_string()),
            idempotency_key: Some("k1".to_string()),
        });
        assert_eq!(opt.protocol, Some("json-rpc/v1".to_string()));
        assert!(opt.idempotent);
        assert_eq!(opt.timeout_ms, Some(5000));
        assert_eq!(opt.routing_key, Some("rk".to_string()));
        assert_eq!(opt.idempotency_key, Some("k1".to_string()));
    }

    #[test]
    fn proxy_error_round_trips_all_variants() {
        assert!(matches!(
            proxy_error_out(HostProxyError::ServiceNotFound("s".to_string())),
            GuestProxyError::ServiceNotFound(s) if s == "s"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::DependencyNotBound("d".to_string())),
            GuestProxyError::DependencyNotBound(s) if s == "d"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::UnsupportedProtocol("p".to_string())),
            GuestProxyError::UnsupportedProtocol(s) if s == "p"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::UnsupportedTarget("t".to_string())),
            GuestProxyError::UnsupportedTarget(s) if s == "t"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::PermissionDenied("u".to_string())),
            GuestProxyError::PermissionDenied(s) if s == "u"
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::Transport("tr".to_string())),
            GuestProxyError::Transport(s) if s == "tr"
        ));
        assert!(matches!(proxy_error_out(HostProxyError::TimedOut), GuestProxyError::TimedOut));
        assert!(matches!(
            proxy_error_out(HostProxyError::Callee(HostCalleeError {
                code: 1,
                message: "m".to_string(),
                data: None,
            })),
            GuestProxyError::Callee(e) if e.code == 1 && e.message == "m" && e.data.is_none()
        ));
        assert!(matches!(
            proxy_error_out(HostProxyError::Internal("i".to_string())),
            GuestProxyError::Internal(s) if s == "i"
        ));
    }
}
