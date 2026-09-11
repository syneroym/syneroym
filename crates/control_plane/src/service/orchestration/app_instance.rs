use super::*;

impl ControlPlaneService {
    /// `adopt`'s read half. `Ok(None)` (not an error)
    /// for a caller with no visibility into the instance -- indistinguish-
    /// able from "no deploy has ever named this instance here", so a
    /// caller with no grant cannot use this to probe for an instance's
    /// existence (the same rule `status`'s `not-found` already follows).
    pub(super) async fn app_instance_management_of_impl(
        &self,
        app_instance_id: String,
        caller: &CallerContext,
    ) -> Result<Option<AppInstanceManagementWire>, String> {
        let held = self.registry.app_instance_management_of(&app_instance_id);
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS)
            && held.as_ref().is_none_or(|m| {
                m.owner_did != caller.caller_did
                    && m.supervisor_did.as_deref() != Some(caller.caller_did.as_str())
            })
        {
            return Ok(None);
        }
        Ok(held.as_ref().map(management_to_wire))
    }

    /// `adopt`'s write half. Subject to the same
    /// four-case rule as every other write, so a racing adopt loses here
    /// rather than at whichever supervisor issues a deploy first. A claim
    /// against an instance with no row at all creates one with
    /// `owner_did = caller`, the same first-write-wins rule `deploy`
    /// uses -- letting a supervisor adopt an instance before its first
    /// deploy lands.
    pub(super) async fn claim_app_instance_impl(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY) {
            return Err(format!(
                "caller {} holds no node-wide orchestrator/deploy on this substrate; claiming an \
                 app instance is node-scoped because the instance spans services",
                caller.caller_did
            ));
        }
        // Generation 0 means unmanaged (the WIT `app-context.generation`
        // doc, and `check_generation`'s own rule): a claim presenting it
        // would persist a row with no supervisor recorded, reporting
        // success while claiming nothing. Refused outright rather than
        // silently accepted -- a real `adopt` always presents `held + 1`
        // (at least 1), so this only rejects a caller invoking the
        // raw verb with a generation that cannot mean what `claim` means.
        if generation == 0 {
            return Err(format!(
                "app instance '{app_instance_id}' cannot be claimed at generation 0; 0 means \
                 unmanaged, so a claim must present a generation of 1 or higher"
            ));
        }
        let management = self.check_generation(&app_instance_id, caller, generation)?;
        self.registry
            .set_app_instance_management(app_instance_id, management)
            .await
            .map_err(|e| e.to_string())
    }

    /// Clears an app instance's management stamp --
    /// `supervisor_did` back to `None`, `generation` back to 0, keeping
    /// `owner_did`. Gated node-wide, not on an invented
    /// `app-instance/<id>` selector: `covers_resource` matches over a
    /// documented selector set with no such segment, and reusing
    /// `app/<app_instance_id>` would put app-instance ids and service ids
    /// in one namespace. The releasing writer must be the current manager
    /// (or ahead of it), so a superseded supervisor cannot release the
    /// instance out from under the live one.
    pub(super) async fn release_app_instance_impl(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY) {
            return Err(format!(
                "caller {} holds no node-wide orchestrator/deploy on this substrate; releasing an \
                 app instance is node-scoped because the instance spans services",
                caller.caller_did
            ));
        }
        // A release against an app instance with no row at all must be a
        // no-op: `check_generation`'s `None` arm exists to let `deploy` and
        // `claim` create a row on first touch, which is right for them but
        // wrong here -- it would let a release mint an ownership row for an
        // instance nobody has ever deployed, blocking a later legitimate
        // deploy from a different caller and leaving an unreachable row
        // behind (no service ever names an instance nobody deployed, so
        // `undeploy_impl`'s cleanup can never find it).
        if self.registry.app_instance_management_of(&app_instance_id).is_none() {
            return Ok(());
        }
        let mut management = self.check_generation(&app_instance_id, caller, generation)?;
        management.supervisor_did = None;
        management.generation = 0;
        self.registry
            .set_app_instance_management(app_instance_id, management)
            .await
            .map_err(|e| e.to_string())
    }
}
