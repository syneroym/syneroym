use super::*;

// ---- signing: in (guest -> host) ----

pub(crate) fn draft_out(v: GuestRecordDraft) -> HostRecordDraft {
    HostRecordDraft {
        version: v.version,
        record_type: v.record_type,
        subject: v.subject,
        payload: v.payload,
        expires_at_secs: v.expires_at_secs,
        supersedes: v.supersedes,
    }
}

pub(crate) fn principal_out(v: GuestPrincipal) -> HostPrincipal {
    match v {
        GuestPrincipal::Service => HostPrincipal::Service,
        GuestPrincipal::Delegated(cert_json) => HostPrincipal::Delegated(cert_json),
    }
}

// ---- signing: out (host -> guest) ----

pub(crate) fn signing_error_guest(v: HostSigningError) -> GuestSigningError {
    match v {
        HostSigningError::NoDelegation(s) => GuestSigningError::NoDelegation(s),
        HostSigningError::InvalidRecord(s) => GuestSigningError::InvalidRecord(s),
        HostSigningError::PermissionDenied => GuestSigningError::PermissionDenied,
        HostSigningError::Internal(s) => GuestSigningError::Internal(s),
    }
}

pub(crate) fn signing_identity_guest(v: HostSigningIdentity) -> GuestSigningIdentity {
    GuestSigningIdentity {
        signing_did: v.signing_did,
        pubkey_hex: v.pubkey_hex,
        owner_did: v.owner_did,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_out_round_trips() {
        let g = GuestRecordDraft {
            version: 1,
            record_type: "listing".to_string(),
            subject: "sub1".to_string(),
            payload: r#"{"price":100}"#.to_string(),
            expires_at_secs: Some(3600),
            supersedes: Some("rec_prev".to_string()),
        };
        let h = draft_out(g);
        assert_eq!(h.version, 1);
        assert_eq!(h.record_type, "listing");
        assert_eq!(h.subject, "sub1");
        assert_eq!(h.payload, r#"{"price":100}"#);
        assert_eq!(h.expires_at_secs, Some(3600));
        assert_eq!(h.supersedes, Some("rec_prev".to_string()));
    }

    #[test]
    fn principal_out_round_trips_all_variants() {
        assert!(matches!(principal_out(GuestPrincipal::Service), HostPrincipal::Service));
        assert!(matches!(
            principal_out(GuestPrincipal::Delegated("cert".to_string())),
            HostPrincipal::Delegated(s) if s == "cert"
        ));
    }

    #[test]
    fn signing_error_guest_round_trips_all_variants() {
        assert!(matches!(
            signing_error_guest(HostSigningError::NoDelegation("a".to_string())),
            GuestSigningError::NoDelegation(s) if s == "a"
        ));
        assert!(matches!(
            signing_error_guest(HostSigningError::InvalidRecord("b".to_string())),
            GuestSigningError::InvalidRecord(s) if s == "b"
        ));
        assert!(matches!(
            signing_error_guest(HostSigningError::PermissionDenied),
            GuestSigningError::PermissionDenied
        ));
        assert!(matches!(
            signing_error_guest(HostSigningError::Internal("c".to_string())),
            GuestSigningError::Internal(s) if s == "c"
        ));
    }

    #[test]
    fn signing_identity_guest_round_trips() {
        let h = HostSigningIdentity {
            signing_did: "did:key:z1".to_string(),
            pubkey_hex: "0102".to_string(),
            owner_did: Some("did:key:zOwner".to_string()),
        };
        let g = signing_identity_guest(h);
        assert_eq!(g.signing_did, "did:key:z1");
        assert_eq!(g.pubkey_hex, "0102");
        assert_eq!(g.owner_did, Some("did:key:zOwner".to_string()));
    }
}
