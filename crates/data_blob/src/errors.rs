use thiserror::Error;

/// Mirrors the `syneroym:blob-store/blob-store` WIT `blob-error` variant
/// 1:1, so host-function and native-dispatch adapters can convert with a
/// plain match.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlobError {
    #[error("blob not found")]
    NotFound,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<object_store::Error> for BlobError {
    fn from(err: object_store::Error) -> Self {
        match err {
            object_store::Error::NotFound { .. } => Self::NotFound,
            other => Self::Internal(other.to_string()),
        }
    }
}
