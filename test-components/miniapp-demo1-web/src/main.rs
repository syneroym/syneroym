#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Mini-app demo main entrypoint
//!
//! Simple guest process to run basic requests in sandbox execution tests.

#[tokio::main]
async fn main() {
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        eprintln!("Failed to install rustls default crypto provider");
        std::process::exit(1);
    }

    miniapp_demo1_web::real_main().await;
}
