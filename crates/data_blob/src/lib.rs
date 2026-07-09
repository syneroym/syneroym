//! Content-addressed, encrypted-at-rest blob object storage for `SynSvc`s.

use std::fmt;

pub mod crypto;
pub mod errors;
pub mod native_types;
pub mod object_store_impl;
pub mod traits;

pub use errors::BlobError;
pub use object_store_impl::ObjectStoreBlobProvider;
pub use traits::{BlobProvider, DownloadSession, UploadSession};

/// Concrete newtype the `syneroym:blob-store/blob-store` `blob-writer`/
/// `blob-reader` WIT resources are mapped to via `with:` in
/// `crates/wit_interfaces/src/host.rs`'s `bindgen!` call -- wasmtime component
/// resources need a nameable host-side representation type, and without an
/// explicit mapping bindgen invents its own opaque marker that trait
/// objects can't satisfy. Mirrors how `wasmtime-wasi` maps its own
/// `input-stream`/`output-stream` resources to concrete types internally.
pub struct HostUploadSession(pub Box<dyn UploadSession>);

impl fmt::Debug for HostUploadSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostUploadSession").finish_non_exhaustive()
    }
}

/// See [`HostUploadSession`].
pub struct HostDownloadSession(pub Box<dyn DownloadSession>);

impl fmt::Debug for HostDownloadSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostDownloadSession").finish_non_exhaustive()
    }
}
