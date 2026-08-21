#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The in-process native implementation of `syneroym-app-host`'s traits.
//! Delegates every call to `HostState`'s existing `Host` impls
//! (`syneroym-sandbox-wasm`) rather than reimplementing any host-capability
//! semantics -- one implementation, two callers (the wasm engine and this
//! shim).

mod convert;
mod factory;
mod host;

pub use factory::NativeHostFactory;
pub use host::{NativeAppHost, NativeBlobReader, NativeBlobWriter};
pub use syneroym_app_host::{ConversationSink, MessageSink};
