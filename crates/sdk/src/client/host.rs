use ed25519_dalek::VerifyingKey;
use syneroym_identity::substrate;
use tracing::debug;

use super::SyneroymClient;

/// Where the host DID comes from. A pure choice, so it can be tested
/// without a network; see `host_key_for` for the lookup.
pub(crate) enum HostChoice<'a> {
    Known(&'a str),
    LookUp,
    Target,
}

/// The host of the target, in this order: a host a registry lookup
/// already named; a host given by `with_host` or by a record; the
/// target itself when the client dialed exactly the target's key;
/// a fresh registry lookup when there is a registry; else the target.
pub(crate) fn choose_host<'a>(
    looked_up: Option<&'a str>,
    expected: Option<&'a str>,
    target: &str,
    dialed: &[u8; 32],
    has_registry: bool,
) -> HostChoice<'a> {
    if let Some(h) = looked_up.or(expected) {
        return HostChoice::Known(h);
    }
    let target_is_dialed = substrate::resolve_did_key(target).is_ok_and(|k| k.as_bytes() == dialed);
    if target_is_dialed || !has_registry { HostChoice::Target } else { HostChoice::LookUp }
}

impl SyneroymClient {
    /// The key the client checks a forwarder's handshake against.
    /// `None` when no rule gives a `did:key`.
    pub(crate) async fn host_key_for(
        &self,
        looked_up: Option<&str>,
        dialed: &[u8; 32],
    ) -> Option<VerifyingKey> {
        let has_registry = !self.registry_url.is_empty();
        let did = match choose_host(
            looked_up,
            self.expected_host.as_deref(),
            &self.service_id,
            dialed,
            has_registry,
        ) {
            HostChoice::Known(h) => h.to_string(),
            HostChoice::Target => self.service_id.clone(),
            HostChoice::LookUp => {
                match self.registry_client().lookup(&self.service_id, false).await {
                    Ok(record) => record.info.substrate_id,
                    Err(e) => {
                        debug!("no record names the host of {}: {e}", self.service_id);
                        self.service_id.clone()
                    }
                }
            }
        };
        substrate::resolve_did_key(&did).ok()
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;

    #[test]
    fn choose_host_rules() {
        let dialed_sk = SigningKey::from_bytes(&[1u8; 32]);
        let dialed_vk = dialed_sk.verifying_key();
        let dialed = *dialed_vk.as_bytes();
        let dialed_did = substrate::derive_did_key(&dialed_vk);

        let other_sk = SigningKey::from_bytes(&[2u8; 32]);
        let other_vk = other_sk.verifying_key();
        let other_did = substrate::derive_did_key(&other_vk);

        let c = choose_host(Some("did:key:zHost1"), None, "did:key:zTarget", &dialed, true);
        assert!(matches!(c, HostChoice::Known("did:key:zHost1")));

        let c = choose_host(Some(&dialed_did), None, &dialed_did, &dialed, true);
        assert!(matches!(c, HostChoice::Known(h) if h == dialed_did));

        let c = choose_host(Some("did:key:zSupervisor"), None, "did:key:zApp", &dialed, true);
        assert!(matches!(c, HostChoice::Known("did:key:zSupervisor")));

        let c = choose_host(None, Some("did:key:zExpected"), "did:key:zTarget", &dialed, true);
        assert!(matches!(c, HostChoice::Known("did:key:zExpected")));

        let c = choose_host(None, None, &dialed_did, &dialed, true);
        assert!(matches!(c, HostChoice::Target));

        let c = choose_host(None, None, &other_did, &dialed, true);
        assert!(matches!(c, HostChoice::LookUp));

        let c = choose_host(None, None, &other_did, &dialed, false);
        assert!(matches!(c, HostChoice::Target));
    }
}
