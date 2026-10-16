use syneroym_core::{
    config::{CoordinatorAccess, CoordinatorRole, SubstrateConfig},
    host_identity::HostIdentity,
};

use super::*;

#[tokio::test]
async fn passport_access_without_tls_fails_even_without_iroh() {
    let mut config = SubstrateConfig::default();
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::Passport,
        iroh: None,
        ..Default::default()
    });
    config.tls = None;

    let host = HostIdentity::unowned("did:key:z6Mku7y9K3o1".to_string());
    let res = EcosystemCoordinator::init(&config, &host).await;
    assert!(res.is_err(), "expected error for passport access without TLS");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("[tls]"), "expected error to name [tls], got: {err}");
}
