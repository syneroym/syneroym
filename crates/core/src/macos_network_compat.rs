//! Keeps a macOS SystemConfiguration defect from crashing the process.
//!
//! When an iroh endpoint binds, `netwatch` (through `netdev`) asks
//! SystemConfiguration for each network interface's localized display name.
//! On macOS 27 that call can abort the process with a SIGSEGV inside
//! CoreFoundation's number formatter. The name is cosmetic: neither `netwatch`
//! nor `iroh` reads it, and the API documents `NULL` as "no name available".
//!
//! The defect is also reported as
//! [tty7 #1234](https://github.com/l0ng-ai/tty7/issues/1234).
//!
//! `install_macos_network_compat!` defines the framework symbol in the
//! calling binary, so the linker resolves that binary's references to our stub
//! instead of the framework's implementation and the suspect Apple code never
//! runs. It must be a macro rather than a function in this library: the
//! definition has to live in the final binary, because a definition inside an
//! `rlib` is not reliably pulled into the link.
//!
//! Invoke it exactly once at module scope in every binary crate that links
//! `netdev` (directly or through `iroh`), including integration tests and
//! benchmarks.

/// Defines `SCNetworkInterfaceGetLocalizedDisplayName` in the calling binary
/// as a stub that always returns `NULL` (a value the API allows).
///
/// Invoke once per binary crate, for example:
///
/// ```ignore
/// syneroym_core::install_macos_network_compat!();
/// ```
#[macro_export]
macro_rules! install_macos_network_compat {
    () => {
        #[cfg(target_os = "macos")]
        mod __macos_network_compat {
            use core::{ffi, ptr};

            #[allow(unsafe_code, reason = "the no_mangle attribute has no safe form")]
            #[unsafe(no_mangle)]
            pub extern "C" fn SCNetworkInterfaceGetLocalizedDisplayName(
                _interface: *const ffi::c_void,
            ) -> *const ffi::c_void {
                ptr::null()
            }
        }
    };
}
