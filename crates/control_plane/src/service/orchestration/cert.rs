use super::*;

/// The ceiling `verify_installed_instance_cert` enforces on an installed
/// instance certificate's lifetime (30 days). A backstop against an
/// unbounded mint, not a forcing function: ADR-0020 §3's attended posture
/// issues deliberately long-lived certificates on an operator's own
/// cadence, and a ceiling tuned for automated renewal would refuse them.
/// Same reasoning, and the same order of magnitude, as
/// `EndpointInfo.not_after`'s own generous bound.
pub(crate) const MAX_INSTANCE_CERT_LIFETIME_SECS: u64 = 30 * 24 * 3600;

impl ControlPlaneService {
    /// ADR-0020 §1's install-time verification of an instance certificate,
    /// shared by every path that installs one. Extracted so `deploy` and
    /// `renew-cert` cannot drift apart on it: two copies of DID and
    /// signature verification silently diverging is a security bug, not a
    /// style one.
    ///
    /// Four checks, in order: the certificate parses, it names
    /// `service_id` as its master, it certifies the key *this* node derives
    /// for *this* caller, and its signature/validity window/scope verify.
    /// Then the lifetime backstop below.
    pub(super) fn verify_installed_instance_cert(
        &self,
        caller_did: &str,
        service_id: &str,
        cert_json: &str,
    ) -> Result<DelegationCertificate, String> {
        let cert = DelegationCertificate::from_json(cert_json)
            .map_err(|e| format!("Invalid instance certificate: {e}"))?;
        // (1) the certificate is for *this* member.
        if cert.master_did != service_id {
            return Err(format!(
                "instance certificate master_did '{}' does not name this deploy's service_id \
                 '{service_id}'",
                cert.master_did
            ));
        }
        // (2) it certifies *this node's* derived key, not some other key
        // the client chose.
        let derived_did = derive_did_key(
            &self.node_identity.derive_service_identity(caller_did, service_id).public_key(),
        );
        if cert.temporary_did != derived_did {
            return Err(format!(
                "instance certificate certifies '{}', not the key this substrate would derive \
                 ('{derived_did}') for this caller and service_id",
                cert.temporary_did
            ));
        }
        // (3) signature, validity window, and the narrow scope.
        cert.verify(service_id, &[SCOPE_SERVICE_INSTANCE])
            .map_err(|e| format!("Invalid instance certificate: {e}"))?;
        // (4) a deliberately generous ceiling on the lifetime, for the same
        // reason `EndpointInfo.not_after` has one: nothing else bounds
        // `expires_at_secs`, so a client-side mint error can produce a
        // certificate valid for years and the near-expiry warning that
        // would have caught it simply never fires. Generous rather than
        // tight on purpose -- the attended posture's certificates are
        // long-lived by design (ADR-0020 §3), and a ceiling tuned for an
        // automated renewal cadence would refuse those operators' own
        // deploys. This catches the unbounded mistake, not the deliberate
        // choice.
        let lifetime = cert.expires_at_secs.saturating_sub(cert.issued_at_secs);
        if lifetime > MAX_INSTANCE_CERT_LIFETIME_SECS {
            return Err(format!(
                "instance certificate lifetime is {lifetime}s, over this substrate's maximum of \
                 {MAX_INSTANCE_CERT_LIFETIME_SECS}s ({} days); reissue it with a shorter window",
                MAX_INSTANCE_CERT_LIFETIME_SECS / 86_400
            ));
        }
        Ok(cert)
    }

    /// Install a freshly-issued instance certificate on an already-deployed
    /// service without reinstalling it -- the certificate-only counterpart
    /// to `restart`, and the only path an unattended renewal has.
    ///
    /// Gated identically to `restart_impl`: the same `orchestrator/deploy`
    /// capability, the same owner-or-node-wide-grantee check, the same
    /// generation gate scoped to an app context, and the same "is this
    /// actually deployed" signal. Without that last one, a capability-
    /// holding caller could register a live native-dispatch entry for a
    /// `service_id` nothing ever deployed: `owner_of` passes vacuously for
    /// an unknown id, and an absent FDAE policy is not an error.
    ///
    /// Rebuilds the service's `SynSvcNativeService` around the new
    /// certificate, mirroring `deploy_with_context`'s own construction site
    /// in full. Without that rebuild the by-value copy the running service
    /// holds keeps the old certificate, and every `RelationshipProof` it
    /// signs afterwards fails verification -- so the rebuild is what makes
    /// renewal a fix rather than a new break.
    pub(super) async fn renew_cert_impl(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `deploy`/`restart`: installing a certificate changes
        // what a service speaks as, which is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may renew its instance certificate"
            ));
        }

        if self.registry.deploy_facts(&service_id).is_none() {
            return Err(format!(
                "'{service_id}' is not deployed here; there is nothing to install a certificate on"
            ));
        }

        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let cert = self.verify_installed_instance_cert(
            &caller.caller_did,
            &service_id,
            &instance_certificate,
        )?;

        // Re-read rather than re-parse from a manifest this call does not
        // have. A stored document that no longer parses aborts here, before
        // anything is installed: falling back to `None` would silently drop
        // row/column filtering for the renewed instance, a materially worse
        // failure than the one `deploy_with_context` already has for the
        // same bad document (it fails the whole call).
        let stored_fdae = self
            .storage_provider
            .load_fdae_policy(&service_id)
            .await
            .map_err(|e| format!("Failed to read the stored FDAE policy: {e}"))?;
        let fdae_policy = match stored_fdae {
            Some(doc) => Some(Arc::new(syneroym_fdae::parse_and_validate(&doc).map_err(|e| {
                tracing::warn!("stored FDAE policy for service {service_id} no longer parses: {e}");
                "the stored FDAE policy no longer validates; renewal aborted rather than \
                 installing a certificate with row filtering silently dropped"
                    .to_string()
            })?)),
            None => None,
        };

        self.registry
            .set_instance_cert(service_id.clone(), cert.clone())
            .await
            .map_err(|e| format!("Instance certificate installation failed: {e}"))?;

        // Mirrors `deploy_with_context`'s own construction site in full --
        // every parameter, not an enumerated subset of it, or the renewed
        // service comes back with dead proxy/authorizer hooks.
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            let native_service = Arc::new(SynSvcNativeService::new(
                service_id.clone(),
                self.key_store.clone(),
                self.storage_provider.clone(),
                self.blob_provider.clone(),
                self.messaging_broker.clone(),
                fdae_policy,
                self.node_identity.clone(),
                &caller.caller_did,
                self.current_service_proxy(),
                self.current_row_authorizer(),
                Some(cert),
            ));
            native_service.set_conversation(self.current_conversation());
            native_service.set_record_signer_from(self);
            native_dispatch.insert(service_id.clone(), native_service as Arc<dyn NativeService>);
            Ok(())
        } else {
            Err(format!(
                "the certificate for '{service_id}' was installed, but the native dispatch \
                 registry is unavailable, so the running service still holds the old one"
            ))
        }
    }
}
