use super::*;

impl ControlPlaneService {
    pub(crate) async fn list_impl(
        &self,
        caller: &CallerContext,
    ) -> Result<Vec<DeployedService>, String> {
        let endpoints = self.registry.get_all_endpoints();
        let mut services: HashMap<String, DeployedService> = HashMap::new();

        for (service_id, interface, endpoint) in endpoints {
            // The native-capability interfaces (data-layer/vault/app-config/
            // blob-store/messaging/http) are host-provided plumbing registered
            // on every deployed service regardless of type -- they must not be
            // mistaken for the service's own declared interfaces, nor
            // influence `endpoint_type` (every deployed service also always
            // has its real wasm/container/tcp endpoint registered).
            if NATIVE_CAPABILITY_INTERFACES.contains(&interface.as_str()) {
                continue;
            }
            let registry = &self.registry;
            let entry = services.entry(service_id.clone()).or_insert_with(|| DeployedService {
                service_id: service_id.clone(),
                interfaces: Vec::new(),
                endpoint_type: match endpoint {
                    SubstrateEndpoint::WasmChannel { .. } => "wasm".to_string(),
                    SubstrateEndpoint::PodmanSocket { .. } => "podman".to_string(),
                    SubstrateEndpoint::NativeHostChannel { .. } => "native".to_string(),
                    SubstrateEndpoint::TcpHostPort { .. } => "tcp".to_string(),
                },
                instance_certificate_expires_at: registry
                    .instance_cert(&service_id)
                    .map(|cert| cert.expires_at_secs),
                visibility: registry.deploy_facts(&service_id).and_then(|f| f.3).map(|v| {
                    match v.as_str() {
                        "public" => WitVisibility::Public,
                        "internal" => WitVisibility::Internal,
                        _ => WitVisibility::Private,
                    }
                }),
            });
            entry.interfaces.push(interface);
        }

        let mut result: Vec<DeployedService> = services.into_values().collect();
        result.sort_by(|a, b| a.service_id.cmp(&b.service_id));

        // Node-wide orchestrator authority sees everything --
        // the substrate owner (a verified `ControllerAgreement` controller;
        // an unowned substrate holds no node-wide authority
        // and so sees nothing here). Checks
        // ORCHESTRATOR_STATUS specifically (unlike the deploy/undeploy
        // checks above): a status-only monitoring grantee is meant to see
        // the list -- that is what the ability names -- without thereby
        // gaining any deploy/undeploy override, which the two checks above
        // enforce independently.
        if self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
            return Ok(result);
        }
        // A service owner sees only their own. `owner_of` == None (deployed
        // by an older binary, or the deploy-crash window) filters OUT: an
        // unattributed app is not "everyone's", and defaulting it visible
        // would make that window a disclosure bug. The substrate owner
        // still sees it above.
        Ok(result
            .into_iter()
            .filter(|s| {
                self.registry.owner_of(&s.service_id).as_deref() == Some(caller.caller_did.as_str())
            })
            .collect())
    }

    /// Per-instance status for a supervisor's poll loop. An empty
    /// `service_ids` means "every service this caller may see", using
    /// `list_impl`'s own visibility filter -- reused verbatim rather than
    /// re-derived, since two independently-maintained visibility rules is
    /// how a disclosure bug gets introduced.
    /// Node facts: gated on node-wide authority, not on seeing any
    /// one service -- what this node can run and where it publishes is a
    /// property of the node, not of a service grant. Split out of
    /// `status_impl` so a caller that only wants these four fields
    /// (`app deploy`'s preflight) never pays `status_impl`'s
    /// per-service phase-check-and-probe cost, which for the node-wide owner
    /// credential means every deployed service on the node.
    pub(crate) fn node_facts_for(&self, caller: &CallerContext) -> Option<NodeFacts> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
            return None;
        }
        let (registry_url, dht_enabled) = match self.endpoint_publisher.get() {
            Some(publisher) => {
                let client = publisher.registry_client();
                (client.registry_url().map(str::to_string), client.dht_enabled())
            }
            // A substrate with no publisher wired (a test harness, or a node
            // with no registry role) reports "unknown", not "none" -- a
            // caller must not read an unwired publisher as a split-registry
            // fleet.
            None => (None, false),
        };
        Some(NodeFacts {
            node_did: self.node_did.clone(),
            service_types: compiled_service_types(),
            registry_url,
            dht_enabled,
        })
    }

    pub(crate) async fn status_impl(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String> {
        // A4-11: `parse_status_params` deliberately accepts anything on the
        // way in (an empty list already means "everything visible", so there
        // is no separate "give me nothing" case to protect), which leaves no
        // size gate on a caller-supplied id list otherwise. Checked before
        // any work happens on the list.
        if service_ids.len() > MAX_STATUS_SERVICE_IDS {
            return Err(format!(
                "status request names {} service ids, over the {MAX_STATUS_SERVICE_IDS} limit",
                service_ids.len()
            ));
        }

        let now = unix_seconds();
        let node = self.node_facts_for(caller);

        let visible = self.list_impl(caller).await?;
        let visible_by_id: HashMap<&str, &DeployedService> =
            visible.iter().map(|d| (d.service_id.as_str(), d)).collect();

        // A4-09 (post-review): a duplicate id in a caller-supplied list is
        // otherwise a target twice over -- A4-05's `join_all` runs both
        // concurrently, so they race the same cache-miss window `probe_cached`
        // has no single-flight for, bypassing the cache entirely rather than
        // the second occurrence reading the first's fresh entry. Deduped
        // here, before the partition, so the raw (pre-dedup) length is still
        // what `MAX_STATUS_SERVICE_IDS` bounds above.
        let mut seen_ids = BTreeSet::new();
        let service_ids: Vec<String> =
            service_ids.into_iter().filter(|id| seen_ids.insert(id.clone())).collect();

        let (targets, named_missing): (Vec<String>, Vec<String>) = if service_ids.is_empty() {
            (visible.iter().map(|d| d.service_id.clone()).collect(), Vec::new())
        } else {
            service_ids.into_iter().partition(|id| visible_by_id.contains_key(id.as_str()))
        };

        // A4-05: every target's phase check and probe run concurrently, not
        // one after another -- a node with several probed services would
        // otherwise serialize their timeouts inside this single RPC, and
        // enough of them could exceed the caller's own deadline, which reads
        // as `SubstrateUnreachable` for every service on an otherwise-healthy
        // node.
        let mut services: Vec<ServiceStatus> =
            futures::future::join_all(targets.iter().map(|service_id| {
                self.service_status_for(service_id, visible_by_id[service_id.as_str()], now)
            }))
            .await;

        // A4-10: a named id that is not visible is always reported
        // `not-found`, never `unauthorized` -- a caller without node-wide
        // `orchestrator/status` must not be able to tell "exists, but I
        // can't see it" from "never deployed" for an id it holds no grant
        // on at all. `readyz`'s rejection text was cited as already leaking
        // this, but it does not: `readyz` returns the identical "holds no
        // orchestrator/status grant" message whether or not the service
        // exists, checked before any existence lookup at all. A caller that
        // *does* hold node-wide status never reaches this branch for an id
        // that actually exists, since `list_impl` already returned it to
        // them above -- so no legitimate caller loses information here.
        for service_id in named_missing {
            services.push(ServiceStatus {
                service_id,
                service_type: None,
                endpoint_type: String::new(),
                app_instance_id: None,
                service_name: None,
                phase: InstancePhase::NotFound,
                probe: ProbeStatus::NotDeclared,
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
                probe_checked_at: None,
                binding_epochs: Vec::new(),
            });
        }

        services.sort_by(|a, b| a.service_id.cmp(&b.service_id));
        Ok(SubstrateStatus { node, checked_at: now, services })
    }

    /// Builds one service's status entry -- phase, probe (which is not
    /// gated by phase), and certificate metadata. Split out of
    /// `status_impl` so every target can be computed concurrently
    /// via `join_all` instead of one after another.
    pub(crate) async fn service_status_for(
        &self,
        service_id: &str,
        dep: &DeployedService,
        now: u64,
    ) -> ServiceStatus {
        let facts = self.registry.deploy_facts(service_id);
        let service_type = facts.as_ref().map(|(t, ..)| t.clone());
        let phase = self.instance_phase(service_id, service_type.as_deref()).await;

        // Phase does NOT gate the probe. A `tcp` service is always
        // `Unknown` -- probing only `Running` would mean a declared probe
        // never runs for exactly the type that has no other signal. It is
        // skipped only where the instance is already known to be down,
        // where it would report a second symptom of one fault.
        let (probe, probe_checked_at) = match &phase {
            InstancePhase::Running | InstancePhase::Unknown(_) => {
                self.probe_cached(service_id, now).await
            }
            _ => (ProbeStatus::NotDeclared, None),
        };

        let cert = self.registry.instance_cert(service_id);
        let app_ctx = self.registry.app_context_of(service_id);

        // Read from the per-dependent persisted row, not the
        // shared resolver entry -- the resolver is keyed
        // `(app-instance-id, service-name)` and is one value per node, so
        // reading it would give every dependent the same answer.
        let binding_epochs = match self.registry.bindings_of(service_id).await {
            Ok(bindings) => bindings
                .into_iter()
                .filter_map(|(name, entry_json)| {
                    match serde_json::from_str::<TopologyEntry>(&entry_json) {
                        Ok(entry) => Some((name, entry.epoch.0)),
                        Err(e) => {
                            tracing::warn!(
                                "stored binding for '{service_id}' dependency '{name}' is \
                                 corrupt: {e}"
                            );
                            None
                        }
                    }
                })
                .collect(),
            Err(e) => {
                tracing::warn!("failed to load bindings for '{service_id}': {e}");
                Vec::new()
            }
        };

        ServiceStatus {
            service_id: service_id.to_string(),
            service_type,
            endpoint_type: dep.endpoint_type.clone(),
            app_instance_id: app_ctx.as_ref().map(|(id, _)| id.clone()),
            service_name: app_ctx.as_ref().map(|(_, name)| name.clone()),
            phase,
            probe,
            instance_certificate_issued_at: cert.as_ref().map(|c| c.issued_at_secs),
            instance_certificate_expires_at: cert.as_ref().map(|c| c.expires_at_secs),
            probe_checked_at,
            binding_epochs,
        }
    }

    /// Derives an [`InstancePhase`] for `service_id` from its recorded
    /// service type. `readyz`'s `is_container` guess is repaired to read
    /// this same fact, so the two surfaces cannot disagree.
    pub(crate) async fn instance_phase(
        &self,
        service_id: &str,
        service_type: Option<&str>,
    ) -> InstancePhase {
        let Some(t) = service_type.and_then(parse_service_type) else {
            // Two cases land here, both correctly "the substrate cannot
            // say": (a) deployed by an older binary -- pre-release, there is
            // no migration, the row appears on the next deploy; (b) the
            // node's own `orchestrator`/`security` endpoints, which
            // `list_impl` includes (it filters `NATIVE_CAPABILITY_INTERFACES`,
            // not the node-level ones) and which no deploy ever created.
            return InstancePhase::Unknown(
                "no service type recorded for this service; redeploy to record it".to_string(),
            );
        };

        // Only the three types a deploy can produce reach here: the wire
        // `service-type` variant has no `native-host` case.
        match t {
            AppServiceType::Wasm => {
                if self.app_sandbox_engine.is_deployed(service_id) {
                    InstancePhase::Running
                } else {
                    InstancePhase::NotRunning(
                        "no compiled component is loaded for this id".to_string(),
                    )
                }
            }
            AppServiceType::Container => {
                match self.podman_sandbox_engine.readyz(service_id).await {
                    Ok(()) => InstancePhase::Running,
                    Err(e) => InstancePhase::NotRunning(e.to_string()),
                }
            }
            // The process runs outside this substrate. A registration is
            // not liveness, and reporting it as `running` would be a lie the
            // supervisor then acts on. A declared probe still runs.
            AppServiceType::Tcp => InstancePhase::Unknown(
                "tcp services run outside this substrate; a declared health check is their only \
                 liveness signal"
                    .to_string(),
            ),
            AppServiceType::NativeHost => InstancePhase::Unknown(
                "native-host services have no deploy-time liveness signal".to_string(),
            ),
        }
    }

    /// Serves a cached probe result within `PROBE_MIN_INTERVAL_SECS`, or runs
    /// a fresh one: a supervisor polling every few seconds must not
    /// turn into probe load on the target, and a wasm `rpc` probe costs a
    /// component instantiation.
    pub(crate) async fn probe_cached(
        &self,
        service_id: &str,
        now: u64,
    ) -> (ProbeStatus, Option<u64>) {
        if let Some(entry) = self.probe_cache.get(service_id)
            && now.saturating_sub(entry.0) < PROBE_MIN_INTERVAL_SECS
        {
            return (entry.1.clone(), Some(entry.0));
        }
        let status = self.run_probe(service_id).await;
        self.probe_cache.insert(service_id.to_string(), (now, status.clone()));
        (status, Some(now))
    }

    /// Runs the declared probe, if any, against the endpoint it names.
    pub(crate) async fn run_probe(&self, service_id: &str) -> ProbeStatus {
        let Some((_, Some(check_json), ..)) = self.registry.deploy_facts(service_id) else {
            return ProbeStatus::NotDeclared;
        };
        let check: WitHealthCheck = match serde_json::from_str(&check_json) {
            Ok(c) => c,
            Err(e) => {
                return ProbeStatus::Failing(format!("stored health check is unreadable: {e}"));
            }
        };

        let interface_name = match &check {
            WitHealthCheck::TcpConnect(p) => p.interface_name.clone(),
            WitHealthCheck::HttpGet(p) => p.interface_name.clone(),
            WitHealthCheck::Rpc(p) => p.interface_name.clone(),
        };
        let Some((endpoint, _)) = self.registry.lookup(service_id, &interface_name) else {
            return ProbeStatus::Failing(format!(
                "no endpoint registered for interface '{interface_name}'"
            ));
        };

        match check {
            WitHealthCheck::TcpConnect(p) => {
                let SubstrateEndpoint::TcpHostPort { host, port } = endpoint else {
                    return ProbeStatus::Failing(format!(
                        "interface '{interface_name}' is not a TCP endpoint"
                    ));
                };
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    tokio::net::TcpStream::connect((host.as_str(), port)),
                )
                .await
                {
                    Ok(Ok(_)) => ProbeStatus::Passing,
                    Ok(Err(e)) => ProbeStatus::Failing(format!("connect failed: {e}")),
                    Err(_) => {
                        ProbeStatus::Failing(format!("connect timed out after {}ms", p.timeout_ms))
                    }
                }
            }
            WitHealthCheck::HttpGet(p) => {
                let SubstrateEndpoint::TcpHostPort { host, port } = endpoint else {
                    return ProbeStatus::Failing(format!(
                        "interface '{interface_name}' is not a TCP endpoint"
                    ));
                };
                let url = format!("http://{host}:{port}{}", p.path);
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    self.http_probe_client.get(&url).send(),
                )
                .await
                {
                    Ok(Ok(resp)) if resp.status().as_u16() == p.expect_status => {
                        ProbeStatus::Passing
                    }
                    Ok(Ok(resp)) => ProbeStatus::Failing(format!(
                        "expected status {}, got {}",
                        p.expect_status,
                        resp.status().as_u16()
                    )),
                    Ok(Err(e)) => ProbeStatus::Failing(format!("http probe failed: {e}")),
                    Err(_) => ProbeStatus::Failing(format!(
                        "http probe timed out after {}ms",
                        p.timeout_ms
                    )),
                }
            }
            WitHealthCheck::Rpc(p) => {
                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: p.method.clone(),
                    params: Value::Array(vec![]),
                    id: Some(Value::from(1)),
                    idempotency_key: None,
                };
                // `execute_probe_json`, not
                // `execute_wasm_json` directly -- bounded by the engine's
                // own `probe_instance_permits`, so a sweep with many
                // `rpc`-probed wasm services cannot request more
                // concurrent component instantiations than the pool can
                // serve (`caller: None` is still a substrate-originated
                // probe, the same choice `ProxyRouter::invoke_local` makes
                // for a guest-to-guest call).
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    self.app_sandbox_engine.execute_probe_json(
                        service_id,
                        &p.interface_name,
                        &request,
                    ),
                )
                .await
                {
                    Ok(Ok(_)) => ProbeStatus::Passing,
                    Ok(Err(e)) => ProbeStatus::Failing(format!("rpc probe failed: {e}")),
                    Err(_) => ProbeStatus::Failing(format!(
                        "rpc probe timed out after {}ms",
                        p.timeout_ms
                    )),
                }
            }
        }
    }
}

/// Service types this build can actually run. Container support is
/// a compile-time Cargo feature and invisible on the wire, which is why the
/// substrate inventory had to trust an operator-typed `capabilities` list
/// (deferred-backlog.md). `tcp` needs no engine and is always available.
pub(crate) fn compiled_service_types() -> Vec<String> {
    let mut types = vec!["tcp".to_string()];
    if cfg!(feature = "app_sandbox") {
        types.push("wasm".to_string());
    }
    if cfg!(feature = "podman_sandbox") {
        types.push("container".to_string());
    }
    types.sort();
    types
}

pub(crate) fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A supervisor polling every few seconds must not turn into probe load on
/// the target substrate (the health-poll-cost budget), and a
/// wasm `rpc` probe costs a component instantiation.
pub(crate) const PROBE_MIN_INTERVAL_SECS: u64 = 5;

/// The most `service_ids` a single `status` call answers. Well above
/// any real fleet a `HealthTarget`/inventory names today; exists only to cap
/// an unbounded, caller-supplied list from any verified caller, not to
/// constrain normal use.
pub(crate) const MAX_STATUS_SERVICE_IDS: usize = 500;
