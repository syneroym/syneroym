//! Applying a compiled deployment plan across one or more substrates, and
//! the per-substrate member-identity minting that multi-substrate placement
//! requires.
//!
//! `PlanApplier` is ADR-0021 §5's narrow "apply this action to that
//! substrate" boundary, introduced here for two reasons:
//! partial-failure behavior is otherwise not testable without killing a live
//! substrate mid-test, and a durable, queue-backed implementation
//! ([`build_durable_actor`]) replaces this trait's body instead of
//! restructuring its callers.

use std::collections::BTreeMap;

use anyhow::Result;
pub use syneroym_app_orchestration::{
    ActionRecord, ActionState, DeploymentJournal, Visibility,
    models::{
        DeploymentPlan, LogicalServiceRef, MemberRef, PlannedService, ServiceId, SubstrateAlias,
    },
};

use crate::mapper::map_deployment_plan_to_wit;

pub mod actor;
pub mod certify;
pub mod types;

#[cfg(test)]
pub(crate) use std::sync::Arc;

pub use actor::*;
pub use certify::*;
#[cfg(test)]
pub(crate) use syneroym_identity::{Identity, substrate};
#[cfg(test)]
pub(crate) use syneroym_rpc::JsonRpcError;
pub use types::*;

#[cfg(test)]
pub(crate) use crate::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, InstanceIdentity,
};

#[cfg(test)]
mod tests;
/// Pairs every service with the target it is placed on, in the plan's own
/// topological order. Fails closed on any alias the caller did not build a
/// target for, before a single deploy call is made -- an unknown alias must
/// never produce a half-applied app.
pub fn resolve_targets<'a>(
    plan: &'a DeploymentPlan,
    targets: &'a BTreeMap<SubstrateAlias, DeployTarget>,
    fallback: Option<&'a DeployTarget>,
) -> Result<Vec<(&'a PlannedService, &'a DeployTarget)>> {
    let mut missing = Vec::new();
    let mut out = Vec::new();
    for svc in &plan.services {
        match (&svc.substrate, fallback) {
            (None, Some(f)) => out.push((svc, f)),
            (None, None) => anyhow::bail!(
                "service '{}' has no placement and no default substrate was supplied",
                svc.logical_ref
            ),
            (Some(alias), _) => match targets.get(alias) {
                Some(t) => out.push((svc, t)),
                None => missing.push((svc.member_ref(), alias.clone())),
            },
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        let list = missing
            .iter()
            .map(|(logical_ref, alias)| format!("{logical_ref} -> '{alias}'"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow::anyhow!("no deploy target built for: {list}"))
    }
}

/// The substrate a member is currently placed on, per the **most recent**
/// row naming it, or `None` if it has never landed or was most recently
/// `REMOVE`d. `landed` must be ordered oldest-first, exactly as
/// `DeploymentJournal::get_completed_actions`/`get_completed_actions_for_
/// instance` return it.
///
/// `member_ref` is a `MemberRef`'s display string: the
/// journal's action rows are keyed per managed member, not per logical
/// service, since two members of one logical service land as two separate
/// placements.
///
/// Shared by `apply_plan`'s resume-skip and `roymctl`'s placement-change
/// refusal (`check_no_placement_change`) so the two cannot read the journal
/// two different ways again: the refusal reads most-recent-row-wins (a
/// `REMOVE` from `app forget` clears it); before this was shared the
/// resume skip did not, so a service `forget`-ten and redeployed under
/// an *unchanged* manifest was wrongly reported "already applied" while
/// running nowhere -- the stale `ADD` row was still present, `.any()` does
/// not care that a `REMOVE` sits after it.
pub fn current_placement<'a>(
    landed: &'a [ActionRecord],
    member_ref: &str,
) -> Option<&'a ActionRecord> {
    match landed.iter().rev().find(|r| r.logical_ref == member_ref) {
        Some(r) if r.action_type == "ADD" => Some(r),
        _ => None,
    }
}

/// Applies one deploy call per (service, substrate), recording a journal
/// action row for each and continuing past a failure rather than aborting
/// the whole app -- a partial deploy is deliberately allowed. A service
/// whose most recent row is `COMPLETED ADD` on the same substrate DID is
/// skipped: a re-run resumes rather than redeploying everything.
pub async fn apply_plan(
    req: ApplyRequest<'_>,
    journal: &DeploymentJournal,
    deployment_id: i64,
) -> Result<ApplyReport> {
    let placed = resolve_targets(req.plan, req.targets, req.fallback)?;
    let completed = journal.get_completed_actions(deployment_id)?;
    let mut report = ApplyReport::default();

    for (svc, target) in placed {
        let l_ref = svc.member_ref().to_string();

        // Keyed on the DID, so an alias re-pointed at a different
        // node correctly redeploys rather than being skipped as already
        // done. `current_placement` reads the most recent row, not just any
        // ADD -- a `REMOVE` from `app forget` must force a redeploy, not a
        // skip, even though an older ADD for the same (ref, DID) pair is
        // still sitting in this record's history.
        if current_placement(&completed, &l_ref)
            .is_some_and(|r| r.substrate_did == target.substrate_did)
        {
            report.skipped.push(svc.member_ref());
            continue;
        }

        let action_id = journal.append_action(
            deployment_id,
            "ADD",
            &l_ref,
            target.alias.as_ref().map(SubstrateAlias::as_str),
            &target.substrate_did,
            ActionState::InProgress,
        )?;

        // A mapping error (an unreadable WASM artifact, an oversized
        // document) is this one service's failure, not the whole app's: the
        // same no-rollback-keep-going rule a substrate-side deploy failure
        // gets.
        let outcome: Result<(), String> = match map_deployment_plan_to_wit(
            req.plan,
            &[svc],
            req.instance_certificates,
            req.registry_certificates,
            req.emit_bindings,
            req.generation,
            req.binding_epochs,
        ) {
            Err(e) => Err(e.to_string()),
            Ok(wit_plan) => target.actor.apply_plan(wit_plan).await,
        };

        match outcome {
            Ok(()) => {
                journal.update_action_state(action_id, ActionState::Completed)?;
                report.deployed.push(svc.member_ref());
            }
            Err(error) => {
                journal.update_action_state(action_id, ActionState::Failed)?;
                report.failures.push(ServiceFailure {
                    member_ref: svc.member_ref(),
                    alias: target.alias.clone(),
                    substrate_did: target.substrate_did.clone(),
                    error,
                });
            }
        }
    }

    Ok(report)
}
