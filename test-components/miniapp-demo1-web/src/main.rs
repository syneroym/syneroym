#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls default crypto provider");

    miniapp_demo1_web::real_main().await;
}
