use super::*;

impl SupervisorService {
    /// Folds `plan.services` against the completed-action journal into a
    /// [`PassPlacements`]. A service with no `current_placement` row is
    /// added to `expected` with empty ids (so the poll reports it
    /// `NotDeployed`) and recorded in `missing_placement`; a placed one
    /// contributes its real ids and, when the row names an alias, a
    /// `did -> alias` entry.
    pub(super) fn resolve_pass_placements(
        landed: &[ActionRecord],
        plan: &DeploymentPlan,
    ) -> PassPlacements {
        let mut placements = PassPlacements {
            expected: Vec::new(),
            missing_placement: BTreeSet::new(),
            did_to_alias: BTreeMap::new(),
        };
        for svc in &plan.services {
            match deploy::current_placement(landed, &svc.member_ref().to_string()) {
                None => {
                    placements.expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                        member_index: svc.member_index,
                    });
                    placements.missing_placement.insert(svc.member_ref().to_string());
                }
                Some(row) => {
                    placements.expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
                        member_index: svc.member_index,
                    });
                    if let Some(alias) = &row.substrate_alias {
                        placements.did_to_alias.insert(row.substrate_did.clone(), alias.clone());
                    }
                }
            }
        }
        placements
    }

    /// One `HealthTarget` per placed substrate DID this pass can name an
    /// inventory entry for. A DID with a connected client polls through it;
    /// one without gets an `UnreachableQuery`, so `poll_once` reports
    /// `SubstrateUnreachable` through its normal error path. A DID whose
    /// alias has no inventory entry at all is a caller-side configuration
    /// gap, not a live outage -- no target is built, and `poll_once`
    /// reports `NoTargetBuilt`/`Unknown` for it.
    pub(super) fn health_targets(
        did_to_alias: &BTreeMap<String, String>,
        inventory: &SupervisorInventory,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<String, HealthTarget> {
        let mut targets: BTreeMap<String, HealthTarget> = BTreeMap::new();
        for (did, alias) in did_to_alias {
            if !inventory.contains_key(alias) {
                continue;
            }
            let query: Arc<dyn StatusQuery> = match clients.get(&SubstrateAlias::new(alias.clone()))
            {
                Some(c) => c.clone() as Arc<dyn StatusQuery>,
                None => Arc::new(UnreachableQuery(format!(
                    "failed to connect to substrate alias '{alias}'"
                ))),
            };
            targets.insert(
                did.clone(),
                HealthTarget {
                    alias: Some(SubstrateAlias::new(alias.clone())),
                    substrate_did: did.clone(),
                    query,
                },
            );
        }
        targets
    }

    pub(super) async fn connected_client(
        &self,
        entry: &SupervisorInventoryEntry,
    ) -> anyhow::Result<SyneroymClient> {
        let identity = Identity::from_bytes(&self.client_identity_bytes);
        let mut client = SyneroymClient::new_with_identity(
            entry.did.clone(),
            entry.api_url.clone().unwrap_or_default(),
            identity,
        )
        .with_registry_dht(self.enable_registry_dht);
        if let Some(token) = &entry.ucan {
            client = client.with_ucan(token.clone());
        }
        client.wait_for_ready(MANAGED_SUBSTRATE_CONNECT_TIMEOUT).await?;
        Ok(client)
    }

    /// Every alias `handle_status` must connect to this pass, deduplicated:
    /// the union of every alias the plan
    /// declares (needed for the generation read, which must reach a
    /// substrate even before anything has landed there) and every alias a
    /// landed placement names (needed for the health sweep). Pulled out
    /// as its own function so the dedup itself -- the whole point of the
    /// fix -- is directly unit-testable without a live substrate.
    pub(super) fn connect_aliases_for_pass(
        plan_aliases: &BTreeSet<String>,
        did_to_alias: &BTreeMap<String, String>,
    ) -> Vec<String> {
        plan_aliases
            .iter()
            .cloned()
            .chain(did_to_alias.values().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Every alias the plan places a service on. Fails closed on a service
    /// with no placement at all: unlike `roymctl app deploy`, the
    /// supervisor has no operator present to supply a `--substrate`
    /// fallback, so an unplaced service can never be applied.
    pub(super) fn placed_aliases(plan: &DeploymentPlan) -> Result<Vec<String>, String> {
        let mut aliases = BTreeSet::new();
        for svc in &plan.services {
            match &svc.substrate {
                Some(alias) => {
                    aliases.insert(alias.as_str().to_string());
                }
                None => {
                    return Err(format!(
                        "service '{}' has no substrate placement; the supervisor has no default \
                         substrate to fall back to",
                        svc.logical_ref
                    ));
                }
            }
        }
        Ok(aliases.into_iter().collect())
    }

    /// Connects one client per placed alias, refusing an alias absent from
    /// the inventory or carrying no credential.
    pub(super) async fn build_clients(
        &self,
        aliases: &[String],
        inventory: &SupervisorInventory,
    ) -> Result<BTreeMap<SubstrateAlias, Arc<SyneroymClient>>, String> {
        let mut clients = BTreeMap::new();
        for alias in aliases {
            let entry = inventory
                .get(alias)
                .ok_or_else(|| format!("no inventory entry for substrate alias '{alias}'"))?;
            if entry.ucan.is_none() {
                return Err(format!(
                    "substrate alias '{alias}' carries no credential (ucan) in the submitted \
                     inventory; the supervisor cannot act on it"
                ));
            }
            let client = self
                .connected_client(entry)
                .await
                .map_err(|e| format!("failed to connect to substrate alias '{alias}': {e}"))?;
            clients.insert(SubstrateAlias::new(alias.clone()), Arc::new(client));
        }
        Ok(clients)
    }

    /// The highest generation any placed, reachable substrate reports
    /// holding for this instance -- best-effort, so one unreachable
    /// substrate cannot hide a real supersession another one reports.
    ///
    /// `aliases` comes from the plan's own declared placement
    /// (`Self::placed_aliases`), not from this supervisor's journal: a
    /// journal-derived set is empty until *this* supervisor has itself
    /// landed a placement, which would make a competing supervisor's
    /// `adopt` on an instance that never finished its first deploy here
    /// undetectable.
    ///
    /// Returns `None`, not `Some(0)`, when not one placed substrate could
    /// be reached and queried -- every failure (no inventory entry,
    /// connect failure, RPC error) previously folded into the same "0" a
    /// substrate with a genuinely empty management row also produces, so
    /// a supervisor that had lost its own `orchestrator/status` grant
    /// reported "not superseded" indefinitely instead of "cannot tell".
    ///
    /// Takes already-connected clients, keyed by alias, rather than
    /// connecting itself -- `handle_status` used to connect to every
    /// substrate twice per call (once for the health sweep, once here),
    /// and this is now the same client set the sweep used.
    ///
    /// Takes `Arc<dyn SubstrateActor>` rather than a concrete
    /// `SyneroymClient` -- callers upcast their real,
    /// connected clients into this shape, and a test substitutes a fake
    /// one instead, so the superseded/skip decision this drives is
    /// testable with no live substrate.
    pub(super) async fn max_held_generation_from_clients(
        app_instance_id: &str,
        aliases: &BTreeSet<String>,
        clients: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
    ) -> Option<u64> {
        let mut held_max = 0u64;
        let mut reached_any = false;
        for alias in aliases {
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else { continue };
            let Ok(generation) = client.held_generation(app_instance_id).await else { continue };
            reached_any = true;
            held_max = held_max.max(generation.unwrap_or(0));
        }
        reached_any.then_some(held_max)
    }

    /// Upcasts a connected client set into the trait-object shape
    /// `max_held_generation_from_clients` takes. Deliberately the plain,
    /// undurable constructor: every actor this builds is used for
    /// exactly one read, `held_generation`, and never for `write_bindings`
    /// -- durability would add a queue key with nothing meaningful to bind
    /// it to and no call that could ever use it.
    pub(super) fn actors_from_clients(
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> {
        clients.iter().map(|(alias, c)| (alias.clone(), deploy::build_actor(c.clone()))).collect()
    }

    /// The durable constructor every other app-supervisor call site with a
    /// real client builds through: `write_bindings` on the returned actor
    /// attempts synchronously first and enqueues onto this supervisor's
    /// own outbox only on a transport failure. Every other action stays
    /// exactly as undurable as `build_actor` would make it -- that
    /// declaration lives inside `DurableActor` itself, not in which call
    /// sites choose this over `build_actor`.
    pub(super) fn durable_actor(
        &self,
        client: Arc<SyneroymClient>,
        app_instance_id: &str,
        logical_ref: &str,
        substrate_did: &str,
    ) -> Arc<dyn SubstrateActor> {
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_did: substrate_did.to_string(),
        };
        let outbox = Arc::new(SupervisorOutbox::new(self.store.queue.clone()));
        deploy::build_durable_actor(
            client,
            substrate_did.to_string(),
            queue_key.to_string(),
            outbox,
        )
    }

    /// Raises or clears `AlertKind::SupervisorSuperseded` from a
    /// `max_held_generation_from_clients` read, and returns whether this
    /// instance is currently superseded (ADR-0021 §4).
    /// Shared by `handle_status` and the loop's own pass so the two cannot
    /// read "superseded" two different ways.
    /// `held_max == None` (nothing reachable) leaves whatever alert state
    /// already exists untouched and reports "not superseded" -- clearing
    /// here would silently un-alert a real supersession just because the
    /// network is flaky right now, and raising would false-alarm on a
    /// transient outage; neither is honest, so this is only logged.
    pub(super) fn update_superseded_alert(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        held_max: Option<u64>,
        generation: u64,
    ) -> Result<bool, String> {
        let Some(held_max) = held_max else {
            tracing::warn!(
                app_instance_id = %app_instance_id,
                "could not reach any placed substrate to check for supersession (matrix row 9); \
                 status cannot confirm this supervisor is still the sole writer"
            );
            return Ok(false);
        };
        let superseded = held_max > generation;
        if superseded {
            self.store
                .alerts
                .raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::SupervisorSuperseded,
                    &format!(
                        "a managed substrate now holds generation {held_max}, higher than this \
                         supervisor's {generation}; another supervisor has adopted this instance"
                    ),
                )
                .map_err(|e| e.to_string())?;
        } else {
            self.store
                .alerts
                .clear(instance_id, None, &self.node_did, AlertKind::SupervisorSuperseded)
                .map_err(|e| e.to_string())?;
        }
        Ok(superseded)
    }

    /// Closes each client's iroh endpoint explicitly, rather than letting
    /// it drop -- a dropped-not-closed `SyneroymClient` is exactly what
    /// iroh logs as "Endpoint dropped without calling `Endpoint::close`.
    /// Aborting ungracefully", and every RPC verb that connects to a
    /// managed substrate used to leave every client it opened for iroh to
    /// clean up on drop. Only closes a client this
    /// call holds the sole `Arc` to -- if something else still references
    /// it, leaving it open is correct, not a leak.
    pub(super) async fn shutdown_clients(clients: impl IntoIterator<Item = Arc<SyneroymClient>>) {
        for mut client in clients {
            if let Some(c) = Arc::get_mut(&mut client) {
                let _ = c.shutdown().await;
            }
        }
    }

    /// `build_clients`' own contract is all-or-nothing (a deploy correctly
    /// wants that), which is wrong for release: an unreachable substrate
    /// must not stop this call from releasing every *other* substrate the
    /// instance is placed on. Connects what it can
    /// and reports the rest as `(alias, reason)` instead of failing the
    /// whole batch on the first one that cannot be reached.
    pub(super) async fn connect_best_effort(
        &self,
        aliases: &[String],
        inventory: &SupervisorInventory,
    ) -> (BTreeMap<SubstrateAlias, Arc<SyneroymClient>>, Vec<(String, String)>) {
        let mut clients = BTreeMap::new();
        let mut failed = Vec::new();
        for alias in aliases {
            // Shutdown must not wait out every remaining alias's own
            // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` -- unlike
            // `queue_worker_tick`'s per-item check, `run()`'s outer
            // `select!` only races cancellation against
            // *waiting for the next tick*, not against a pass already in
            // flight, so without a check here a pass stuck connecting to
            // one unreachable alias silently drags the whole shutdown out
            // by however many alias timeouts remain.
            if self.cancellation_token.is_cancelled() {
                break;
            }
            let Some(entry) = inventory.get(alias) else {
                failed.push((
                    alias.clone(),
                    "no inventory entry for this substrate alias".to_string(),
                ));
                continue;
            };
            if entry.ucan.is_none() {
                failed.push((
                    alias.clone(),
                    "substrate carries no credential (ucan) in the submitted inventory".to_string(),
                ));
                continue;
            }
            tokio::select! {
                () = self.cancellation_token.cancelled() => break,
                result = self.connected_client(entry) => match result {
                    Ok(client) => {
                        clients.insert(SubstrateAlias::new(alias.clone()), Arc::new(client));
                    }
                    Err(e) => failed.push((alias.clone(), e.to_string())),
                },
            }
        }
        (clients, failed)
    }
}
