//! Vault entry names the substrate keeps for itself. A guest cannot read
//! or write them through the vault interface.

/// Every name with this prefix belongs to the substrate.
pub const RESERVED_PREFIX: &str = "syneroym.";

/// The member's mailbox secret key (32-byte seed).
pub const MAILBOX_SECRET: &str = "syneroym.mailbox-secret";

#[must_use]
pub fn is_reserved(name: &str) -> bool {
    name.starts_with(RESERVED_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mailbox_secret_name_is_reserved() {
        assert!(is_reserved(MAILBOX_SECRET));
    }

    #[test]
    fn ordinary_names_are_not_reserved() {
        assert!(!is_reserved("member-x"));
        assert!(!is_reserved("api_key"));
        assert!(!is_reserved("syneroym"));
        assert!(!is_reserved("my.syneroym.key"));
    }
}
