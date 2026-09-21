use super::*;

impl SupervisorService {
    pub(in crate::service) async fn handle_export_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse export-master params: {e}"))
        })?;
        let path = self
            .vault
            .export_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse {
            payload: serde_json::to_value(path.to_string_lossy().into_owned())
                .unwrap_or(Value::Null),
        })
    }

    pub(in crate::service) async fn handle_import_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse import-master params: {e}"))
        })?;
        self.vault
            .import_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "imported"}) })
    }

    /// The DID `revoke-instance` actually anchors as revoked. Read from
    /// the hosting substrate rather than a stored table: the substrate is
    /// the authority on what key is actually installed.
    ///
    /// `instance_did` is what *this caller* (this supervisor) would
    /// derive -- correct for the certify flow that reads it before
    /// anything is installed, wrong here whenever the installed
    /// certificate was minted for a different caller (a member deployed
    /// by an operator and only later adopted, not yet redeployed).
    /// Revoking the derived DID in that case anchors a key nothing
    /// presents, while the key actually in use stays fully trusted -- so
    /// this prefers `installed_temporary_did`, the substrate's ground
    /// truth for what is installed right now, and only falls back to the
    /// derived DID when nothing is installed yet (nothing to read, so the
    /// prospective key is the closest thing to "the key this placement
    /// would use"). A free function of the RPC's answer alone, so the
    /// choice is directly testable without a live client.
    pub(in crate::service) fn select_revocation_did(
        identity: syneroym_sdk::InstanceIdentity,
    ) -> String {
        identity.installed_temporary_did.unwrap_or(identity.instance_did)
    }

    /// Revoke one placed member's instance key: append its derived DID to
    /// the master anchor's revoked list, then record the placement revoked
    /// so nothing mints it a fresh certificate afterwards.
    ///
    /// Under the instance lock for the whole verb, the same discipline
    /// every other instance-scoped write follows. Without it, this and a
    /// resident pass's renewal of the same member race: the pass could mint
    /// and install a fresh certificate in the gap between the anchor write
    /// and the exclusion write landing, which is precisely the window this
    /// verb exists to close.
    ///
    /// Order matters. The local exclusion is written **after** the anchor
    /// publish succeeds, so a failed publish leaves the placement under
    /// ordinary management rather than half-revoked -- excluded from
    /// renewal here while still fully trusted by every consumer.
    pub(in crate::service) async fn handle_revoke_instance(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, logical_ref): (String, String) = serde_json::from_value(params)
            .map_err(|e| {
                RpcError::InvalidParams(format!("failed to parse revoke-instance params: {e}"))
            })?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;

        // Checked here as well as inside `record_revocation`, so a node
        // with no registry refuses before spending a round trip resolving
        // an instance identity it can do nothing with.
        if self.anchor_writer.is_none() {
            return Err(RpcError::InternalError(
                "this supervisor's node has no registry configured (substrate.registry_url), so \
                 it cannot publish a revocation; a revocation nothing can resolve is not a \
                 revocation"
                    .to_string(),
            ));
        }

        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let svc =
            plan.services.iter().find(|s| s.member_ref().to_string() == logical_ref).ok_or_else(
                || {
                    RpcError::InvalidParams(format!(
                        "app instance '{app_instance_id}' has no member '{logical_ref}' in its \
                         stored plan"
                    ))
                },
            )?;
        let alias = svc.substrate.as_ref().ok_or_else(|| {
            RpcError::InternalError(format!("member '{logical_ref}' has no substrate placement"))
        })?;
        let entry = inventory.get(alias.as_str()).ok_or_else(|| {
            RpcError::InternalError(format!("no inventory entry for substrate alias '{alias}'"))
        })?;

        // `select_revocation_did`'s own doc explains the choice below.
        let mut client = self
            .connected_client(entry)
            .await
            .map_err(|e| RpcError::InternalError(format!("failed to reach '{alias}': {e}")))?;
        let identity = client.instance_identity(svc.service_id.as_str()).await;
        let _ = client.shutdown().await;
        let identity = identity.map_err(|e| {
            RpcError::InternalError(format!(
                "failed to resolve the instance identity for '{logical_ref}': {e}"
            ))
        })?;
        let instance_did = Self::select_revocation_did(identity);

        self.record_revocation(
            &app_instance_id,
            &logical_ref,
            svc.logical_ref.service_name.as_str(),
            svc.member_index,
            &instance_did,
        )
        .await
        .map_err(RpcError::InternalError)?;

        Ok(NativeResponse {
            payload: serde_json::json!({
                "status": "revoked",
                "instance_did": instance_did,
                "note": "the member's process is still running; undeploy it separately if that is \
                         intended",
            }),
        })
    }

    /// `revoke-instance`'s two writes, once the instance DID is known.
    /// Split from the verb so the ordering below is exercisable without a
    /// live substrate answering `resolve-instance-identity` -- which is the
    /// only reason the verb needs a network at all.
    ///
    /// The anchor publish comes first and the local exclusion only after it
    /// succeeds. Reversed, a failed publish would leave the placement
    /// half-revoked: excluded from renewal here, while every consumer still
    /// fully trusts the key -- so it would quietly age out instead of
    /// failing closed, which is the opposite of what was asked for.
    pub(in crate::service) async fn record_revocation(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        service_name: &str,
        member_index: u32,
        instance_did: &str,
    ) -> Result<(), String> {
        let writer = self.anchor_writer.as_ref().ok_or_else(|| {
            "this supervisor's node has no registry configured (substrate.registry_url), so it \
             cannot publish a revocation"
                .to_string()
        })?;
        let master =
            keys::master_for_member(&self.vault, app_instance_id, service_name, member_index)
                .await
                .map_err(|e| e.to_string())?;
        writer
            .revoke_instance(&master, instance_did)
            .await
            .map_err(|e| format!("failed to publish the revocation: {e}"))?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.store
            .revoke_placement(app_instance_id, logical_ref, now as i64)
            .map_err(|e| e.to_string())
    }
}
