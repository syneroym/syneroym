use super::*;

impl ControlPlaneService {
    /// The service_id-path-safety, reservation, ownership/takeover, and
    /// deploy-capability checks every deploy must pass before any other
    /// work starts. Pure reads -- nothing here writes anything, so
    /// returning `Err` leaves no state to roll back.
    pub(super) fn validate_deploy_admission(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // `service_id` is joined verbatim into `hosted_apps_dir/<service_id>
        // .json` (write, and also delete on a private redeploy) -- reject
        // anything that could walk that join out of the directory before it
        // is used for anything, including the ownership/capability checks
        // that follow.
        if !is_safe_service_id_for_path(service_id) {
            return Err(format!(
                "service_id '{service_id}' is not a valid deploy target: it must be non-empty and \
                 contain no '/', '\\\\', or '..' -- it is joined into a stored-record filename"
            ));
        }
        // A deploy may not claim a `service_id` this substrate already uses
        // as a fixed `native_dispatch` key: the node's own DID (claiming it
        // hijacks every `orchestrator`/`security` call this node ever
        // receives) or the literal `"supervisor"`/auth alias (whose vault a
        // deploy under that name would also open). Neither `SERVICE_ID_
        // REGEX` nor `validate_service_id` reserves either string -- both
        // are ordinary, deployable-looking ids otherwise.
        if service_id == self.node_did
            || service_id == SUPERVISOR_RESERVED_SERVICE_ID
            || service_id == AUTH_RESERVED_SERVICE_ID
        {
            return Err(format!(
                "service_id '{service_id}' is reserved for this substrate's own dispatch and \
                 cannot be deployed to"
            ));
        }

        // A service_id already owned by someone else may not be re-deployed
        // into. An unowned substrate holds no node-wide orchestrator
        // authority, so this always enforces the takeover check there --
        // only an owned substrate's owner can override it.
        //
        // TOCTOU note (reviewed, accepted): this read and the terminal
        // `set_owner` write far below are separated by the whole deploy
        // body, not atomic. Two concurrent *first* deploys of the same
        // brand-new `service_id` from different DIDs can both observe
        // `owner_of == None` and both proceed -- whichever `set_owner` call
        // lands last wins attribution. This cannot defeat an *existing*
        // owner's protection (a service that already has a recorded owner
        // is rejected deterministically regardless of timing), so it is an
        // attribution race on a service_id nobody owns yet, not a
        // takeover-check bypass. Not fixed here: closing it fully needs a
        // per-service_id lock or an atomic claim-then-verify around the
        // entire (non-atomic, pre-existing) deploy flow, which is a larger
        // change than this one.
        if let Some(existing) = self.registry.owner_of(service_id)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {existing}; redeploy must come from its owner \
                 or a substrate owner"
            ));
        }

        // Tier-1 deploy admission. The caller must hold `orchestrator/
        // deploy` covering this app. No owner/unowned branch and no
        // separate substrate-owner bypass here: a bare `substrate:<node>`
        // capability (the owner's `substrate/admin`) is `is_substrate_
        // scope`, so `grants` wildcards the resource and only `entails` has
        // to hold -- that passes here for free. An app-scoped grantee is
        // prefix-covered instead. One check, two principals, no branch.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }
        Ok(())
    }

    /// ADR-0020 §1's install-time instance-certificate verification, plus
    /// ADR-0018 §4's publication/visibility check -- both placed before any
    /// artifact work runs, so a bad certificate or an unauthorized
    /// visibility declaration is rejected at deploy rather than discovered
    /// later as a routing failure.
    pub(super) fn validate_deploy_certs_and_visibility(
        &self,
        service_id: &str,
        caller: &CallerContext,
        manifest: &DeployManifest,
    ) -> Result<(Option<DelegationCertificate>, AppVisibility), String> {
        let installed_instance_cert = match &manifest.instance_certificate {
            Some(cert_json) => Some(self.verify_installed_instance_cert(
                &caller.caller_did,
                service_id,
                cert_json,
            )?),
            None => None,
        };
        let validated_visibility = validate_publication(
            service_id,
            manifest.config.visibility,
            manifest.registry_certificate.as_deref(),
        )?;
        Ok((installed_instance_cert, validated_visibility))
    }

    /// ADR-0021 §2: validates and prepares `app_context`'s bindings, and
    /// persists the ADR-0021 §4 generation-gate stamp immediately (that
    /// write records *who is writing*, not what was installed, so it is not
    /// behind the defer-until-everything-succeeds rule the returned
    /// `PreparedAppContext` itself is subject to -- see `install_app_context`
    /// and the call site's own comment).
    pub(super) async fn prepare_app_context_for_deploy(
        &self,
        app_context: &Option<AppContext>,
        caller: &CallerContext,
    ) -> Result<Option<PreparedAppContext>, String> {
        let Some(ctx) = app_context else { return Ok(None) };
        // Validate before anything touches storage, so a later read of
        // these rows can only fail on real corruption rather than on
        // something a deploy caller sent.
        let instance_id = AppInstanceId::try_new(&ctx.app_instance_id)
            .map_err(|e| format!("app context names an invalid app instance id: {e}"))?;
        LogicalServiceName::try_new(&ctx.service_name)
            .map_err(|e| format!("app context names an invalid service name: {e}"))?;

        // An app instance's first successful deploy becomes its owner
        // (first-write-wins, the same shape `service_id` ownership uses in
        // `validate_deploy_admission`, including that check's own TOCTOU
        // note). Without this, any caller authorized to deploy *some*
        // service could name an existing, unrelated app instance in its own
        // `app_context` and overwrite the bindings that instance's other
        // services resolve.
        if let Some(existing) =
            self.registry.app_instance_management_of(&ctx.app_instance_id).map(|m| m.owner_did)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "app instance '{}' is owned by {existing}; a deploy that joins it must come from \
                 its owner or a substrate owner",
                ctx.app_instance_id
            ));
        }

        let management = self.check_generation(&ctx.app_instance_id, caller, ctx.generation)?;
        self.registry
            .set_app_instance_management(ctx.app_instance_id.clone(), management)
            .await
            .map_err(|e| e.to_string())?;

        let mut bindings = Vec::with_capacity(ctx.bindings.len());
        for binding in &ctx.bindings {
            let (dependency_name, entry) = prepare_binding(binding, &ctx.app_instance_id)?;
            bindings.push((binding.dependency_name.clone(), dependency_name, entry));
        }

        Ok(Some(PreparedAppContext {
            instance_id,
            raw_instance_id: ctx.app_instance_id.clone(),
            raw_service_name: ctx.service_name.clone(),
            bindings,
        }))
    }

    /// Whether this deploy is a retry after a lost response: same caller,
    /// byte-identical content already installed and running.
    /// `full_deploy_completed` is the witness that *this* process (not just
    /// a past one) already registered this service's routing -- those route
    /// tables are process-local and empty on every boot, so a fresh boot's
    /// redeploy always falls through here and re-registers everything, even
    /// when the persisted `manifest_hash` still matches. See the field's own
    /// doc comment.
    pub(super) async fn deploy_is_redundant_noop(
        &self,
        service_id: &str,
        caller: &CallerContext,
        service_type: AppServiceType,
        incoming_hash: &str,
    ) -> bool {
        self.full_deploy_completed.contains_key(service_id)
            && self.registry.deploy_facts(service_id).and_then(|(_, _, hash, _)| hash).as_deref()
                == Some(incoming_hash)
            && self.registry.owner_of(service_id).as_deref().is_none_or(|o| o == caller.caller_did)
            && !matches!(
                self.instance_phase(service_id, Some(service_type_str(service_type))).await,
                InstancePhase::NotRunning(_) | InstancePhase::NotFound
            )
    }
}
