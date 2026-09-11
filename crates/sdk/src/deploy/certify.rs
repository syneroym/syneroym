//! Certificate issuance and member endpoint record generation.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use ed25519_dalek::VerifyingKey;
use syneroym_app_orchestration::{
    Visibility,
    models::{DeploymentPlan, LogicalServiceRef, ServiceId, SubstrateAlias},
};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};

use super::SubstrateActor;
use crate::{InstanceIdentity, SyneroymClient};
/// The attended posture's default certificate lifetime for a plan-deploy-time
/// certification.
pub const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

/// Queries `client`'s substrate for the instance key it would derive for
/// `service_id` under the connecting caller's identity, and issues a
/// `service-instance`-scoped certificate over it from `master`. This is the
/// full round trip: one read-only RPC, one local signature, no install call
/// -- installation happens at deploy.
///
/// **Bound to this client, not just this master.** The substrate derives the
/// key from its own node identity *and* the calling DID, so a certificate
/// minted through one client is rejected at deploy by any other substrate,
/// and by the same substrate reached as a different caller.
pub async fn certify_instance(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let identity = client
        .instance_identity(service_id)
        .await
        .context("failed to query the substrate for its derived instance identity")?;
    certificate_over_instance_identity(master, service_id, &identity, expires_hours)
}

/// Issue a `record-signing`-scoped certificate from `master` over the node's
/// derived signing key for `service_id`.
pub async fn certify_record_signing(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let master_did = substrate::derive_did_key(&master.public_key());
    let signing_identity = client
        .signing_identity(service_id)
        .await
        .context("failed to query the substrate for its derived signing identity")?;

    match signing_identity.owner_did.as_deref() {
        Some(owner) if owner == master_did => {}
        Some(owner) => anyhow::bail!(
            "master identity resolves to {master_did}, which does not match service owner {owner} \
             -- a certificate for this pair would be rejected at sign time"
        ),
        None => anyhow::bail!(
            "service {service_id} has no recorded owner -- cannot certify signing identity"
        ),
    }

    let temp_pubkey = substrate::resolve_did_key(&signing_identity.signing_did)
        .context("failed to resolve signing DID key")?;

    let cert = DelegationCertificate::issue(
        master,
        temp_pubkey,
        expires_hours * 3600,
        syneroym_identity::delegation::SCOPE_RECORD_SIGNING.to_string(),
    )?;
    Ok(cert)
}

/// The same round trip as [`certify_instance`], through the
/// [`SubstrateActor`] boundary -- what an unattended renewal takes, since
/// the supervisor's loop holds actors rather than raw clients and its
/// renewal control flow has to be testable against a fake substrate.
pub async fn certify_instance_via_actor(
    actor: &Arc<dyn SubstrateActor>,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let identity = actor.instance_identity(service_id).await.map_err(|e| {
        anyhow::anyhow!("failed to query the substrate for its derived instance identity: {e}")
    })?;
    certificate_over_instance_identity(master, service_id, &identity, expires_hours)
}

/// The shared half of both `certify_instance` forms: everything after the
/// substrate has answered with the key it derives. One definition, so a
/// renewal and a deploy-time mint cannot drift apart on what they check
/// before signing.
///
/// **Bound to the querying client, not just this master.** The substrate
/// derives the key from its own node identity *and* the calling DID, so a
/// certificate minted through one client is rejected at install by any
/// other substrate, and by the same substrate reached as a different caller.
fn certificate_over_instance_identity(
    master: &Identity,
    service_id: &str,
    identity: &InstanceIdentity,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let master_did = substrate::derive_did_key(&master.public_key());
    if master_did != service_id {
        anyhow::bail!(
            "master identity resolves to {master_did}, which does not match service_id \
             {service_id} -- a certificate for this pair would be rejected at install time"
        );
    }

    let pubkey_bytes = hex::decode(&identity.pubkey_hex)
        .context("substrate returned an invalid hex-encoded instance pubkey")?;
    let pubkey_array: [u8; 32] = pubkey_bytes.try_into().map_err(|_| {
        anyhow::anyhow!("substrate returned an instance pubkey of the wrong length")
    })?;
    let pubkey = VerifyingKey::from_bytes(&pubkey_array)
        .context("substrate returned an invalid ed25519 instance pubkey")?;

    DelegationCertificate::issue(
        master,
        pubkey,
        expires_hours * 3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
}

/// ADR-0018 §4: the registry record a placed member's declared
/// visibility produces, signed and serialized -- or `None` for `private`,
/// which mints no record at all. Pure and network-free (unlike
/// [`certify_instance`], which needs the substrate's own derived key), so it
/// is the one piece of [`certify_placed_members`] directly unit testable
/// without a live substrate, and the one both it and any test harness
/// building the same shape by hand should share rather than re-derive.
pub fn member_registry_record(
    visibility: Visibility,
    service_id: &str,
    substrate_id: &str,
    master: &Identity,
    not_after: u64,
) -> Result<Option<String>> {
    let is_private = match visibility {
        Visibility::Private => return Ok(None),
        Visibility::Internal => true,
        Visibility::Public => false,
    };
    let record = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: substrate_id.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private,
        ttl: None,
        not_after,
        generation: 0,
    }
    .sign(master)?;
    Ok(Some(serde_json::to_string(&record)?))
}

/// Mints, per placed member, the instance certificate its hosting substrate
/// will accept and the endpoint record that points at that substrate.
///
/// Returns `(instance_certificates, registry_certificates)`, both keyed by
/// the member master `ServiceId`, ready for `ApplyRequest`.
pub async fn certify_placed_members(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    // `None` when every service is placed by alias.
    fallback: Option<&Arc<SyneroymClient>>,
    expires_hours: u64,
) -> Result<(BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>)> {
    // Two services sharing one master would need two endpoint records under
    // one service_id pointing at different substrates -- a permanent
    // compare-and-swap fight at the registry. Impossible from today's
    // compiler (one master per PlannedService), so this is an assertion, not
    // a supported case.
    let mut seen: BTreeMap<&ServiceId, &LogicalServiceRef> = BTreeMap::new();
    for svc in &plan.services {
        if let Some(prev) = seen.insert(&svc.service_id, &svc.logical_ref) {
            anyhow::bail!(
                "member master {} is placed twice ({}, {})",
                svc.service_id,
                prev,
                svc.logical_ref
            );
        }
    }

    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = masters.get(&svc.service_id).ok_or_else(|| {
            anyhow::anyhow!("no member master resolved for service {}", svc.service_id)
        })?;
        let client = match (&svc.substrate, fallback) {
            (Some(alias), _) => clients
                .get(alias)
                .ok_or_else(|| anyhow::anyhow!("no client built for substrate alias '{alias}'"))?,
            (None, Some(f)) => f,
            (None, None) => anyhow::bail!(
                "service '{}' has no placement and no default substrate was supplied",
                svc.logical_ref
            ),
        };

        let cert = certify_instance(client, master, svc.service_id.as_str(), expires_hours).await?;
        certs.insert(svc.service_id.clone(), cert.to_json()?);

        let not_after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
        if let Some(record_json) = member_registry_record(
            svc.config.visibility,
            svc.service_id.as_str(),
            client.service_id(),
            master,
            not_after,
        )? {
            records.insert(svc.service_id.clone(), record_json);
        }
    }

    Ok((certs, records))
}
