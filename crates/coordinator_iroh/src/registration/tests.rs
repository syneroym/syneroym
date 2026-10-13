use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use syneroym_community_registry::EcosystemRegistry;
use syneroym_core::config::{ServiceRegistryRole, SubstrateConfig};
use tokio::{sync::watch, task, time};

use super::*;

#[tokio::test(start_paused = true)]
async fn registration_repeats_on_its_own_timer() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = watch::channel(false);

    let register = {
        let calls = Arc::clone(&calls);
        move || {
            let calls = Arc::clone(&calls);
            async move { calls.fetch_add(1, Ordering::SeqCst) > 0 }
        }
    };

    let handle = tokio::spawn(keep_registered(register, tx));

    while calls.load(Ordering::SeqCst) < 1 {
        task::yield_now().await;
    }
    assert!(!*rx.borrow());

    time::advance(REGISTRATION_RETRY - Duration::from_secs(1)).await;
    task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::SeqCst) < 2 {
        task::yield_now().await;
    }
    assert!(*rx.borrow());

    time::advance(REGISTRATION_REFRESH - Duration::from_secs(1)).await;
    task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    time::advance(Duration::from_secs(1)).await;
    while calls.load(Ordering::SeqCst) < 3 {
        task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    handle.abort();
}

#[tokio::test]
async fn second_registration_is_accepted_by_registry() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut config_r = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("data_r"),
        app_data_dir: temp_dir.path().join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        ..Default::default()
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    let secret_key = SecretKey::generate(&mut rand::rng());
    let reg = Registration::new(
        r_url.clone(),
        secret_key.clone(),
        Some("https://relay.example.com".to_string()),
        RetryPolicy::default(),
    );

    let ok1 = reg.register_with_retries().await;
    assert!(ok1, "first registration should succeed");

    let did = coordinator_did(&secret_key);
    let reg_client = RegistryClient::new(false, Some(r_url));
    let lookup1 = reg_client.lookup(&did, false).await?;
    assert_eq!(lookup1.info.service_id, did);

    time::sleep(Duration::from_millis(50)).await;

    let ok2 = reg.register_with_retries().await;
    assert!(ok2, "second registration should be accepted as refresh");

    let lookup2 = reg_client.lookup(&did, false).await?;
    assert_eq!(lookup2.info.service_id, did);
    assert!(lookup2.info.not_after >= lookup1.info.not_after);

    registry_r.shutdown().await?;
    Ok(())
}
