use super::*;

impl SupervisorService {
    pub(super) fn signal_str(signal: &Signal) -> &'static str {
        match signal {
            Signal::Healthy => "healthy",
            Signal::SubstrateUnreachable(_) => "substrate-unreachable",
            Signal::InstanceNotRunning(_) => "instance-not-running",
            Signal::ProbeFailing(_) => "probe-failing",
            Signal::Unknown(_) => "unknown",
            Signal::NotDeployed => "not-deployed",
        }
    }

    pub(super) fn signal_detail(signal: &Signal) -> String {
        match signal {
            Signal::Healthy | Signal::NotDeployed => String::new(),
            Signal::SubstrateUnreachable(d)
            | Signal::InstanceNotRunning(d)
            | Signal::ProbeFailing(d)
            | Signal::Unknown(d) => d.clone(),
        }
    }

    /// Runs a fresh health sweep inside the RPC rather than reading rows
    /// nothing writes -- this read surface is not idle, it just isn't on a
    /// resident timer.
    pub(super) async fn handle_status(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse status params: {e}")))?;

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

        let instance_id = AppInstanceId::try_new(app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let PassPlacements { expected, missing_placement, did_to_alias } =
            Self::resolve_pass_placements(&landed, &plan);

        // One client set for the whole call, shared by the health sweep
        // and the generation read below -- `handle_status`
        // used to connect to every substrate twice. The connected set is
        // the union of every alias the plan declares (needed for the
        // generation read, which must reach a substrate even before
        // anything has landed there) and every alias a landed placement
        // names (needed for the health sweep).
        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(&plan).unwrap_or_default().into_iter().collect();
        let connect_aliases = Self::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, &inventory).await;
        // These used to be discarded entirely. An unreachable substrate
        // is already visible another way (the
        // health sweep reports it as a fault for a service placed
        // there), but an alias with no inventory entry or no credential
        // is a configuration problem the health sweep cannot see at
        // all, since it never gets far enough to try connecting.
        for (alias, reason) in &failed {
            tracing::warn!(
                app_instance_id,
                alias,
                reason,
                "failed to connect to a substrate this pass needs"
            );
        }

        let targets = Self::health_targets(&did_to_alias, &inventory, &clients);

        let report = health::poll_once(&targets, &expected).await;
        // Drops `targets`' `Arc<dyn StatusQuery>` clones so `clients`
        // holds the sole remaining `Arc` to each client, which is what
        // lets `shutdown_clients` reach `Arc::get_mut` below.
        drop(targets);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        // A planned service the journal has never recorded landed is a
        // deploy failure the sweep cannot see (it has no
        // `service_id`/`substrate_did` to probe at all, reported as
        // `NotDeployed`, deliberately not a fault).
        // The supervisor holds the plan, so it knows the difference
        // between "not in the plan" and "in the plan and missing" --
        // reuses `InstanceNotRunning` rather than a fifth `AlertKind`,
        // since the operator reads this as the same problem.
        //
        // Keyed on `NEVER_LANDED_SUBSTRATE_DID`, not the empty string:
        // every planned-but-unlanded service also appears in `report.
        // services` as `Signal::NotDeployed` with `substrate_did == ""`,
        // and `record_report`'s own per-service loop unconditionally
        // *clears* `(instance, logical_ref, "", InstanceNotRunning)` for
        // exactly that case (no active fault to report) -- raising under
        // that same empty-string key would have it cleared on every
        // subsequent call, then re-raised here as a "new" incident every
        // time. A distinct sentinel dodges that loop, but is then itself
        // invisible to `record_report`'s *other* pass -- the "this
        // (logical_ref, substrate_did) pair left the sweep entirely, so
        // clear it" cleanup -- which would otherwise clear this alert
        // every single call, for the identical reason in reverse.
        // `extra_live_pairs` is exactly the exemption that cleanup needs.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        // Same call, same constant, as the resident loop's own.
        let mut opened = health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
            now,
            &extra_live_pairs,
            SUPERVISOR_CERT_ALERT_POLICY,
        )
        .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Folded into `opened` (not published separately) so the publish
        // call below sees every alert this pass newly raised, not only
        // the ones `record_report` itself knows about.
        self.sync_never_landed_alerts(&instance_id, &plan, &missing_placement, &mut opened)?;

        // Publication happens here, in `record_report`'s caller, over the
        // newly-opened list above -- every store write
        // that could add to it has already committed, so a publish
        // failure below can never lose an alert by construction. Never
        // propagated with `?`: an unreachable/slow MQTT broker must not
        // fail the whole `status` call.
        self.publish_opened_alerts(&app_instance_id, &opened).await;

        // ADR-0021 §4: a substrate reporting a higher generation than this
        // supervisor holds means a second supervisor
        // has adopted the instance. Checked against every substrate the
        // *plan* places a service on, not `did_to_alias` above (which only
        // covers substrates this supervisor's own journal already shows a
        // landed placement on, and is empty until the first one lands) --
        // see `max_held_generation_from_clients`'s own doc.
        let held_max = Self::max_held_generation_from_clients(
            &app_instance_id,
            &plan_aliases,
            &Self::actors_from_clients(&clients),
        )
        .await;
        let superseded = self
            .update_superseded_alert(&instance_id, &app_instance_id, held_max, state.generation)
            .map_err(RpcError::InternalError)?;

        // Closed once, at the end, now that both the health sweep and the
        // generation read above are done with them.
        Self::shutdown_clients(clients.into_values()).await;

        let services = self.managed_services_from_report(&app_instance_id, &report);
        let overall_state = self.overall_managed_state(
            &instance_id,
            state.retired,
            state.paused,
            superseded,
            &report,
            &missing_placement,
        )?;

        // Computed before `state.app_master_did` is moved into the
        // literal below -- keyed by the app master DID, not the instance
        // id, so a handover's new DID starts at "never refreshed" rather
        // than inheriting the old DID's stamp.
        let app_record_expires_at = (!state.app_master_did.is_empty())
            .then(|| self.store.last_tier1_refresh(&state.app_master_did).unwrap_or(None))
            .flatten()
            .map(|at| (at as u64).saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS));

        let status = InstanceStatus {
            app_instance_id: app_instance_id.clone(),
            state: overall_state,
            generation: state.generation,
            supervisor_did: self.node_did.clone(),
            // An earlier version ran no reconcile loop, so this used to be
            // permanently `None`. It is not `Some(now)` either -- that
            // reported every instance as having just reconciled, even one
            // that never has. The loop now stamps `last_reconciled` at
            // the end of every pass it actually runs for this instance;
            // `status`'s own on-demand sweep, right here, deliberately
            // does not count as one.
            last_reconciled_at: self.last_reconciled.get(&app_instance_id).map(|v| *v as u64),
            services,
            // Read off the store's own written epoch and this pass's
            // observed one, per declared dependency.
            bindings: self.binding_convergence_rows(&app_instance_id, &plan, &report),
            delivery_note: "delivery is best-effort synchronous; a converged status is not a \
                            durability guarantee"
                .to_string(),
            // Reads the same table `apply_with_clients`
            // already consults on every write pass, so a revocation is
            // visible here the moment it lands, not only once some other
            // change triggers a write that reaches the member.
            revoked_placements: self
                .store
                .revoked_placements(&app_instance_id)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            // Read from the stored row only, never the vault -- a locked
            // vault is the ordinary state of a
            // freshly-booted supervisor, and this field must stay readable
            // through it. Empty means "never adopted under A7", mapped to
            // `None` here so a caller does not have to know `""` is a
            // sentinel.
            app_master_did: (!state.app_master_did.is_empty()).then_some(state.app_master_did),
            // ADR-0022 §2: derived from the last successful refresh this
            // supervisor stamped, not read back from the
            // registry -- the deadline an operator has before a locked
            // vault (or a pause) costs this instance's cross-app
            // discoverability, made visible here alongside `VaultLocked`.
            app_record_expires_at,
        };
        Ok(NativeResponse { payload: serde_json::to_value(status).unwrap_or(Value::Null) })
    }

    /// Raises `InstanceNotRunning` for every planned service the journal
    /// has never recorded landed, and clears it for every one that now
    /// has a placement. Keyed on `NEVER_LANDED_SUBSTRATE_DID`, folded into
    /// `opened` so the caller publishes it with every other alert this
    /// pass raised. `?`-propagating: unlike the resident loop's own
    /// analogous sync, a store error here fails the whole `status` call.
    fn sync_never_landed_alerts(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        missing_placement: &BTreeSet<String>,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> RpcResult<()> {
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if missing_placement.contains(&l_ref) {
                if self
                    .store
                    .alerts
                    .raise(
                        instance_id,
                        Some(&l_ref),
                        None,
                        NEVER_LANDED_SUBSTRATE_DID,
                        AlertKind::InstanceNotRunning,
                        "planned but never deployed; the supervisor holds no completed placement \
                         for this service",
                    )
                    .map_err(|e| RpcError::InternalError(e.to_string()))?
                {
                    opened.push((AlertKind::InstanceNotRunning, l_ref));
                }
            } else {
                self.store
                    .alerts
                    .clear(
                        instance_id,
                        Some(&l_ref),
                        NEVER_LANDED_SUBSTRATE_DID,
                        AlertKind::InstanceNotRunning,
                    )
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// One `ManagedService` row per service the sweep reported, carrying
    /// its signal and the restart-attempt count from the `remediation`
    /// table. `Ok(None)` (no restart ever attempted) and a read failure
    /// both fall back to 0, the correct value for "no attempts recorded".
    fn managed_services_from_report(
        &self,
        app_instance_id: &str,
        report: &health::HealthReport,
    ) -> Vec<ManagedService> {
        report
            .services
            .iter()
            .map(|s| ManagedService {
                logical_ref: s.member_ref().to_string(),
                service_id: s.service_id.clone(),
                substrate_alias: s
                    .alias
                    .as_ref()
                    .map(SubstrateAlias::to_string)
                    .unwrap_or_default(),
                substrate_did: s.substrate_did.clone(),
                signal: Self::signal_str(&s.signal).to_string(),
                detail: Self::signal_detail(&s.signal),
                restart_attempts: self
                    .store
                    .remediation_state(app_instance_id, &s.member_ref().to_string())
                    .ok()
                    .flatten()
                    .map_or(0, |r| r.attempts),
            })
            .collect()
    }

    /// The instance's single reported state, in precedence order:
    /// `retired`/`superseded`/`paused` are stored facts; `Applying` is a
    /// reconcile in flight (ranked before the health verdict, since a
    /// verdict computed from a half-applied plan is less useful than "ask
    /// again"); then `Active` only when the sweep found no fault, every
    /// planned service has a placement, and no `BindingConflict` alert
    /// stands; otherwise `Degraded`.
    fn overall_managed_state(
        &self,
        instance_id: &AppInstanceId,
        retired: bool,
        paused: bool,
        superseded: bool,
        report: &health::HealthReport,
        missing_placement: &BTreeSet<String>,
    ) -> RpcResult<ManagedState> {
        let is_applying = self
            .store
            .journal
            .get_latest(instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .is_some_and(|r| r.state == DeploymentState::Applying);
        // Read off the *active* alert set rather than "any unconverged
        // row": a push that just landed cleanly reads as unconverged on
        // `binding-epochs` for up to one poll interval simply because the
        // observed epoch has not been re-polled yet, and that must not
        // flap the instance `Degraded` on every ordinary change.
        let has_binding_conflict = self
            .store
            .alerts
            .active(instance_id)
            .map(|active| active.iter().any(|a| a.kind == AlertKind::BindingConflict))
            .unwrap_or(false);

        Ok(if retired {
            ManagedState::Retired
        } else if superseded {
            ManagedState::Superseded
        } else if paused {
            ManagedState::Paused
        } else if is_applying {
            ManagedState::Applying
        } else if report.faults().is_empty()
            && missing_placement.is_empty()
            && !has_binding_conflict
        {
            ManagedState::Active
        } else {
            ManagedState::Degraded
        })
    }

    pub(super) async fn handle_alerts(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, all): (String, bool) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse alerts params: {e}")))?;
        let instance_id = AppInstanceId::try_new(app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let rows = if all {
            self.store.alerts.all(&instance_id)
        } else {
            self.store.alerts.active(&instance_id)
        }
        .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let alerts: Vec<Alert> = rows
            .into_iter()
            .map(|r| Alert {
                logical_ref: r.logical_ref,
                substrate_did: r.substrate_did,
                kind: r.kind.to_string(),
                detail: r.detail,
                first_seen_at: r.first_seen_at,
                last_seen_at: r.last_seen_at,
                cleared_at: r.cleared_at,
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(alerts).unwrap_or(Value::Null) })
    }

    /// Every item belonging to this instance still in the outbox -- pending
    /// or claimed, not yet dead-lettered: `roymctl supervisor outbox`'s
    /// own listing, and what an end-to-end test needs to assert the item
    /// is actually queued rather than only inferring it from alerts or
    /// `is_converged`.
    pub(super) async fn handle_outbox(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse outbox params: {e}")))?;

        let rows = self.store.queue.all().map_err(|e| RpcError::InternalError(e.to_string()))?;
        let items: Vec<OutboxItem> = rows
            .into_iter()
            .filter_map(|item| {
                let key: QueueKey = item.queue_key.parse().ok()?;
                (key.app_instance_id == app_instance_id).then_some(OutboxItem {
                    id: item.id as u64,
                    logical_ref: key.logical_ref,
                    substrate_did: key.substrate_did,
                    attempts: item.attempts,
                })
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(items).unwrap_or(Value::Null) })
    }

    /// Every dead letter belonging to this instance, oldest first --
    /// `roymctl supervisor dead-letters`'s own listing.
    pub(super) async fn handle_dead_letters(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse dead-letters params: {e}"))
        })?;

        let rows =
            self.store.queue.dead_letters().map_err(|e| RpcError::InternalError(e.to_string()))?;
        let dead_letters: Vec<DeadLetter> = rows
            .into_iter()
            .filter_map(|d| {
                let key: QueueKey = d.queue_key.parse().ok()?;
                (key.app_instance_id == app_instance_id).then_some(DeadLetter {
                    id: d.id as u64,
                    logical_ref: key.logical_ref,
                    substrate_did: key.substrate_did,
                    attempts: d.attempts,
                    last_error: d.last_error,
                    created_at: d.created_at,
                })
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(dead_letters).unwrap_or(Value::Null) })
    }

    /// Re-enqueues a dead letter -- it never executes inline.
    /// Refuses a dead letter belonging to a different app instance, rather
    /// than silently replaying it: the caller named one instance, and a
    /// wrong id must not act on someone else's queued work.
    pub(super) async fn handle_replay(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, dead_letter_id): (String, u64) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse replay params: {e}")))?;
        let instance_id = AppInstanceId::try_new(app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let rows =
            self.store.queue.dead_letters().map_err(|e| RpcError::InternalError(e.to_string()))?;
        // Both branches below are the caller naming a dead letter that is
        // not theirs to replay -- an unknown id and one that belongs to a
        // different instance are the same class of mistake as a malformed
        // parameter, not a server problem, so both answer `InvalidParams`.
        // Neither names which other instance
        // (if any) actually owns the id: that would confirm to a caller
        // that an id exists under someone else's instance, which an
        // instance-scoped admin grant should not leak.
        let Some(row) = rows.iter().find(|d| d.id as u64 == dead_letter_id) else {
            return Err(RpcError::InvalidParams(format!(
                "no dead letter with id {dead_letter_id} for app instance '{app_instance_id}'"
            )));
        };
        let key: QueueKey = row.queue_key.parse().map_err(|e| {
            RpcError::InternalError(format!("dead letter carries an unparseable queue key: {e}"))
        })?;
        if key.app_instance_id != app_instance_id {
            return Err(RpcError::InvalidParams(format!(
                "no dead letter with id {dead_letter_id} for app instance '{app_instance_id}'"
            )));
        }

        self.store
            .queue
            .replay(dead_letter_id as i64, outbox::now_ms())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        self.clear_delivery_exhausted_if_empty(&instance_id, &key);
        Ok(NativeResponse { payload: serde_json::json!({"status": "replayed"}) })
    }

    /// Every schedule this instance declares, in logical-ref order:
    /// `roymctl supervisor schedules`'s own listing.
    /// Left-joins the stored plan's declarations against `schedule_states`
    /// -- a declared schedule with no state row yet (never evaluated)
    /// appears with `evaluated-at: 0`.
    pub(super) async fn handle_schedules(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse schedules params: {e}"))
        })?;

        let Some(state) =
            self.store.get(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?
        else {
            return Ok(NativeResponse {
                payload: serde_json::to_value(Vec::<ScheduledTask>::new()).unwrap_or(Value::Null),
            });
        };
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let schedule_states = self
            .store
            .schedule_states(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let tasks: Vec<ScheduledTask> = Self::declared_schedules(&plan)
            .into_iter()
            .map(|(logical_ref, sched)| {
                let recorded = schedule_states.get(&logical_ref);
                ScheduledTask {
                    logical_ref: logical_ref.clone(),
                    cron: sched.cron.clone(),
                    interface: sched.interface.to_string(),
                    method: sched.method.clone(),
                    evaluated_at: recorded.map_or(0, |s| s.evaluated_at),
                    last_run_at: recorded.and_then(|s| s.last_run_at),
                    last_member_index: recorded.and_then(|s| s.last_member_index),
                    last_error: recorded.and_then(|s| s.last_error.clone()),
                }
            })
            .collect();
        Ok(NativeResponse { payload: serde_json::to_value(tasks).unwrap_or(Value::Null) })
    }
}
