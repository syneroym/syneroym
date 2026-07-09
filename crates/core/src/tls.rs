use std::{fmt, path::PathBuf};

use axum_server::tls_rustls::RustlsConfig;
use tracing::{error, info};

pub struct TlsCertLoader {
    config: RustlsConfig,
}

impl fmt::Debug for TlsCertLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsCertLoader").finish()
    }
}

impl TlsCertLoader {
    pub async fn new(cert_path: PathBuf, key_path: PathBuf) -> anyhow::Result<Self> {
        let config = RustlsConfig::from_pem_file(&cert_path, &key_path).await?;
        Ok(Self { config })
    }

    pub fn config(&self) -> RustlsConfig {
        self.config.clone()
    }

    pub fn spawn_watcher(&self, cert_path: PathBuf, key_path: PathBuf) {
        let config = self.config.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let mut sig = match signal(SignalKind::user_defined1()) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to register SIGUSR1 handler: {:?}", e);
                        return;
                    }
                };

                info!("Registered SIGUSR1 handler for TLS certificate hot-reload");
                while sig.recv().await.is_some() {
                    info!(
                        "Received SIGUSR1. Reloading TLS certificates from {:?} and {:?}",
                        cert_path, key_path
                    );
                    if let Err(e) = config.reload_from_pem_file(&cert_path, &key_path).await {
                        error!("Failed to reload TLS certificates: {:?}", e);
                    } else {
                        info!("Successfully reloaded TLS certificates");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                warn!("SIGUSR1 hot-reload is only supported on Unix systems");
            }
        });
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::fs;

    use rustls::crypto::ring;
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn test_tls_cert_loader_missing_files() {
        let dir = tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        let loader = TlsCertLoader::new(cert_path, key_path).await;
        assert!(loader.is_err());
    }

    #[tokio::test]
    async fn test_tls_cert_loader_valid_files() {
        let _ = ring::default_provider().install_default();
        let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cert_path = base_dir.join("../../test-components/miniapp-demo1-web/src/test_cert.pem");
        let key_path = base_dir.join("../../test-components/miniapp-demo1-web/src/test_key.pem");

        let loader = TlsCertLoader::new(cert_path, key_path).await;
        assert!(loader.is_ok(), "Expected valid TLS loading to succeed: {:?}", loader.err());
    }

    #[tokio::test]
    async fn test_tls_cert_loader_hot_reload() {
        let _ = ring::default_provider().install_default();
        let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src_cert_path =
            base_dir.join("../../test-components/miniapp-demo1-web/src/test_cert.pem");
        let src_key_path =
            base_dir.join("../../test-components/miniapp-demo1-web/src/test_key.pem");

        let dir = tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        fs::copy(&src_cert_path, &cert_path).unwrap();
        fs::copy(&src_key_path, &key_path).unwrap();

        let loader = TlsCertLoader::new(cert_path.clone(), key_path.clone()).await.unwrap();

        // Modify files slightly or rewrite them to trigger actual reload
        fs::copy(&src_cert_path, &cert_path).unwrap();
        fs::copy(&src_key_path, &key_path).unwrap();

        // Directly verify reloading works without sending OS-level signal
        let reload_res = loader.config().reload_from_pem_file(&cert_path, &key_path).await;
        assert!(reload_res.is_ok(), "Expected reload to succeed: {:?}", reload_res.err());
    }
}
