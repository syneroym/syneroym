use super::*;

impl SupervisorService {
    /// Whether `old` and `new` (the same member, before and after a
    /// resubmit) differ only in which member DIDs a dependency resolves to
    /// -- a membership change in one of `new`'s dependencies, and nothing
    /// else about this member itself. `logical_ref` is
    /// already guaranteed equal: `Reconciler::diff_plans` matches `old` and
    /// `new` by `MemberRef`, which includes it. Also requires `schedule` to
    /// be unchanged: a simultaneous schedule edit must not
    /// classify as membership-only, or the schedule change is silently
    /// dropped from the plan this pass records.
    pub(in crate::service) fn only_resolved_dependencies_changed(
        old: &PlannedService,
        new: &PlannedService,
    ) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.schedule == new.schedule
            && old.resolved_dependencies != new.resolved_dependencies
    }

    /// Whether `old` and `new` differ only in `schedule`.
    /// A schedule-only edit must not redeploy the service -- the
    /// substrate has no use for the change at all (`ServiceSpec.schedule`'s
    /// own doc) -- and has nothing to push either, since it names no
    /// substrate-visible fact.
    pub(in crate::service) fn only_schedule_changed(
        old: &PlannedService,
        new: &PlannedService,
    ) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.resolved_dependencies == new.resolved_dependencies
            && old.schedule != new.schedule
    }

    /// Splits a diff's `Update` actions into (a) members no caller should
    /// redeploy this pass and (b) the subset of those that need a binding
    /// push. The two are not the same set: a member
    /// whose only change is its schedule must not be redeployed (the
    /// substrate has no use for the change) and has nothing to push
    /// either -- so it joins the exclusion set but never the push list.
    ///
    /// The asymmetry in the landed-placement check below is deliberate: a
    /// membership push needs a substrate to push *to*, so a never-landed
    /// member falls through to the redeploy path. A schedule exclusion
    /// needs no substrate, so it applies whether or not the member has
    /// landed.
    ///
    /// Shared by the loop's write phase (`reconcile_instance_pass`) and an
    /// operator-triggered apply (`apply_with_membership_pushes`, under
    /// `handle_submit`/`deploy_submission`) so both make the identical
    /// redeploy-vs-push-vs-exclude call for the identical diff -- fixing
    /// this classification for one path and not the other is exactly the
    /// gap an earlier review round found.
    pub(in crate::service) fn classify_update_actions(
        landed: &[ActionRecord],
        actions: &[ReconcileAction],
    ) -> (BTreeSet<String>, Vec<(PlannedService, String)>) {
        let mut redeploy_exclusions = BTreeSet::new();
        let mut push_candidates = Vec::new();
        for action in actions {
            if let ReconcileAction::Update { old, new } = action {
                let member_ref = new.member_ref().to_string();
                if Self::only_schedule_changed(old, new) {
                    redeploy_exclusions.insert(member_ref);
                    continue;
                }
                let landed_row = Self::only_resolved_dependencies_changed(old, new)
                    .then(|| deploy::current_placement(landed, &member_ref))
                    .flatten();
                if let Some(row) = landed_row {
                    redeploy_exclusions.insert(member_ref);
                    push_candidates.push(((**new).clone(), row.substrate_did.clone()));
                }
            }
        }
        (redeploy_exclusions, push_candidates)
    }

    /// The loop's redeploy work list (D-A5c-2/D-A5c-3/D-A5c-21): the diff's
    /// `Add` and `Update` actions -- a plan-level change -- plus
    /// `missing_placement`, a service the current sweep finds with no
    /// landed placement at all, which a content-unchanged diff against an
    /// older `Active` snapshot cannot see on its own (D-A5c-10's gap).
    /// `Remove` is not work: a plan-level removal is never undeployed here,
    /// only raised as `OrphanedService` by the caller.
    ///
    /// `redeploy_exclusions` comes from `classify_update_actions`, and this
    /// is the loop's half of applying it -- the half a test can reach
    /// without a substrate to deploy at. Its counterpart on the operator
    /// path is the `retain` in `apply_with_membership_pushes`.
    pub(in crate::service) fn redeploy_work_list(
        missing_placement: &BTreeSet<String>,
        actions: &[ReconcileAction],
        redeploy_exclusions: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut needs_work = missing_placement.clone();
        for action in actions {
            let member_ref = match action {
                ReconcileAction::Add(svc) => svc.member_ref().to_string(),
                ReconcileAction::Update { new, .. } => new.member_ref().to_string(),
                ReconcileAction::Remove(_) => continue,
            };
            if !redeploy_exclusions.contains(&member_ref) {
                needs_work.insert(member_ref);
            }
        }
        needs_work
    }

    /// Every schedule a plan declares, keyed by logical ref. A schedule is
    /// identical across a logical service's members, so the first member
    /// carrying one decides it for the whole group; `BTreeMap` keeps both
    /// the pass and the `schedules` listing in a deterministic order.
    /// Shared by the two so a schedule the pass acts on and a schedule the
    /// operator is shown can never be different sets.
    pub(in crate::service) fn declared_schedules(
        plan: &DeploymentPlan,
    ) -> BTreeMap<String, &ScheduleSpec> {
        let mut groups: BTreeMap<String, &ScheduleSpec> = BTreeMap::new();
        for svc in &plan.services {
            if let Some(sched) = &svc.schedule {
                groups.entry(svc.logical_ref.to_string()).or_insert(sched);
            }
        }
        groups
    }
}
