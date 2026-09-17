use super::*;

// ---- blob-store: out (host -> guest) ----

pub(crate) fn blob_error_out(v: HostBlobError) -> GuestBlobError {
    match v {
        HostBlobError::NotFound => GuestBlobError::NotFound,
        HostBlobError::QuotaExceeded => GuestBlobError::QuotaExceeded,
        HostBlobError::Internal(s) => GuestBlobError::Internal(s),
    }
}

// ---- messaging: out (host -> guest) ----

pub(crate) fn msg_error_out(v: HostMessagingError) -> GuestMessagingError {
    match v {
        HostMessagingError::PermissionDenied => GuestMessagingError::PermissionDenied,
        HostMessagingError::Internal(s) => GuestMessagingError::Internal(s),
    }
}

// ---- app-config: out (host -> guest) ----

pub(crate) fn config_error_out(v: HostConfigError) -> GuestConfigError {
    match v {
        HostConfigError::Internal(s) => GuestConfigError::Internal(s),
    }
}

// ---- vault: out (host -> guest) ----

pub(crate) fn vault_error_out(v: HostVaultError) -> GuestVaultError {
    match v {
        HostVaultError::NotFound => GuestVaultError::NotFound,
        HostVaultError::PermissionDenied => GuestVaultError::PermissionDenied,
        HostVaultError::Internal(s) => GuestVaultError::Internal(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_error_round_trips_all_variants() {
        assert!(matches!(blob_error_out(HostBlobError::NotFound), GuestBlobError::NotFound));
        assert!(matches!(
            blob_error_out(HostBlobError::QuotaExceeded),
            GuestBlobError::QuotaExceeded
        ));
        assert!(matches!(
            blob_error_out(HostBlobError::Internal("z".to_string())),
            GuestBlobError::Internal(s) if s == "z"
        ));
    }

    #[test]
    fn messaging_error_round_trips_all_variants() {
        assert!(matches!(
            msg_error_out(HostMessagingError::PermissionDenied),
            GuestMessagingError::PermissionDenied
        ));
        assert!(matches!(
            msg_error_out(HostMessagingError::Internal("w".to_string())),
            GuestMessagingError::Internal(s) if s == "w"
        ));
    }

    #[test]
    fn config_error_round_trips_all_variants() {
        assert!(matches!(
            config_error_out(HostConfigError::Internal("i".to_string())),
            GuestConfigError::Internal(s) if s == "i"
        ));
    }

    #[test]
    fn vault_error_round_trips_all_variants() {
        assert!(matches!(vault_error_out(HostVaultError::NotFound), GuestVaultError::NotFound));
        assert!(matches!(
            vault_error_out(HostVaultError::PermissionDenied),
            GuestVaultError::PermissionDenied
        ));
        assert!(matches!(
            vault_error_out(HostVaultError::Internal("i".to_string())),
            GuestVaultError::Internal(s) if s == "i"
        ));
    }
}
