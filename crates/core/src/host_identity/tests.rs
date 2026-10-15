use syneroym_identity::substrate::{SubstrateIdentityState, SubstrateIdentityStatus};

use super::*;

#[test]
fn verified_controller_root_is_controller() {
    let state = SubstrateIdentityState {
        did: "did:key:zNode".to_string(),
        controller: Some("did:key:zController".to_string()),
        status: SubstrateIdentityStatus::Verified,
    };
    let host = HostIdentity::new(&state);
    assert_eq!(host.node_did, "did:key:zNode");
    assert_eq!(host.controller_status, SubstrateIdentityStatus::Verified);
    assert_eq!(host.verified_controller.as_deref(), Some("did:key:zController"));
    assert_eq!(host.passport_root(), "did:key:zController");

    let info = host.identity_info();
    assert_eq!(info.did, "did:key:zNode");
    assert_eq!(info.controller_status, "verified");
}

#[test]
fn unverified_controller_root_is_node_did() {
    let state = SubstrateIdentityState {
        did: "did:key:zNode".to_string(),
        controller: Some("did:key:zController".to_string()),
        status: SubstrateIdentityStatus::Unverified,
    };
    let host = HostIdentity::new(&state);
    assert_eq!(host.node_did, "did:key:zNode");
    assert_eq!(host.controller_status, SubstrateIdentityStatus::Unverified);
    assert_eq!(host.verified_controller, None);
    assert_eq!(host.passport_root(), "did:key:zNode");

    let info = host.identity_info();
    assert_eq!(info.did, "did:key:zNode");
    assert_eq!(info.controller_status, "unverified");
}

#[test]
fn no_controller_root_is_node_did() {
    let state = SubstrateIdentityState {
        did: "did:key:zNode".to_string(),
        controller: None,
        status: SubstrateIdentityStatus::None,
    };
    let host = HostIdentity::new(&state);
    assert_eq!(host.node_did, "did:key:zNode");
    assert_eq!(host.controller_status, SubstrateIdentityStatus::None);
    assert_eq!(host.verified_controller, None);
    assert_eq!(host.passport_root(), "did:key:zNode");

    let info = host.identity_info();
    assert_eq!(info.did, "did:key:zNode");
    assert_eq!(info.controller_status, "none");

    let unowned = HostIdentity::unowned("did:key:zUnowned".to_string());
    assert_eq!(unowned.passport_root(), "did:key:zUnowned");
    assert_eq!(unowned.identity_info().controller_status, "none");

    let random = HostIdentity::random_unowned();
    assert_eq!(random.controller_status, SubstrateIdentityStatus::None);
    assert_eq!(random.passport_root(), &random.node_did);
}
