//! KeyStore service for managing Key Encryption Keys (KEK) and Data Encryption
//! Keys (DEK).

pub mod key_store;

pub use key_store::KeyStore;

#[derive(thiserror::Error, Debug)]
pub enum KeyStoreError {
    #[error("Encryption KEK is required but has not been injected")]
    KekRequired,
    #[error("KEK has already been injected and cannot be re-injected without re-authentication")]
    KekAlreadyInjected,
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Crypto error: {0}")]
    Crypto(String),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, KeyStoreError>;
