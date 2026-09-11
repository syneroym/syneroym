use super::*;

/// Maps the wire `topology-mode` variant to the app model's `TopologyMode`
/// -- the inverse of `syneroym_sdk::mapper`'s `map_mode`.
pub(crate) fn map_topology_mode(mode: WitTopologyMode) -> AppTopologyMode {
    match mode {
        WitTopologyMode::Singleton => AppTopologyMode::Singleton,
        WitTopologyMode::Redundant => AppTopologyMode::Redundant,
        WitTopologyMode::Sharded => AppTopologyMode::Sharded,
    }
}

/// Wire `service-type` variant -> the app model's `ServiceType`.
/// Only the discriminant matters here; the payload is what the deploy already
/// used. The wire variant has no `native-host` case -- only the three types a
/// deploy can actually produce reach here.
pub(crate) const fn app_service_type(t: &WitServiceType) -> AppServiceType {
    match t {
        WitServiceType::Wasm(_) => AppServiceType::Wasm,
        WitServiceType::Container(_) => AppServiceType::Container,
        WitServiceType::Tcp(_) => AppServiceType::Tcp,
    }
}

/// The content of a `registry_certificate` blob that actually describes the
/// deployed service, as distinct from the parts that change on every mint
/// regardless of whether anything else did:
/// `SignedEndpointInfo.info.not_after` (`SystemTime::now()` plus a fixed
/// window) and `pkarr_packet_hex` (its own embedded signing timestamp) both
/// churn on every `certify_placed_members` call. Every other `EndpointInfo`
/// field belongs here, `generation` (ADR-0022 §2) included: every member
/// record today passes `0`, so its absence is latent, not yet a wrong
/// answer -- but a real generation change is a real content change, and
/// omitting it here would let this dedup treat two different generations
/// as identical the day one publisher's does. Falls back to hashing the raw
/// string on a parse failure -- that can only make the dedup *more*
/// conservative (a parse failure never equals anything, including itself
/// byte-for-byte reapplied), never less safe.
pub(crate) fn stable_registry_certificate_for_hash(json: &str) -> String {
    match serde_json::from_str::<SignedEndpointInfo>(json) {
        Ok(signed) => serde_json::to_string(&(
            &signed.info.service_id,
            &signed.info.substrate_id,
            &signed.info.endpoint_type,
            &signed.info.mechanisms,
            &signed.info.nickname,
            signed.info.is_private,
            &signed.info.ttl,
            signed.info.generation,
        ))
        .unwrap_or_else(|_| json.to_string()),
        Err(_) => json.to_string(),
    }
}

/// Whether `service_id` is safe to join, unescaped, into a filename under
/// `hosted_apps_dir` -- `deploy_with_context`'s certificate write/delete,
/// and `undeploy_impl`'s own delete of the same file. Real service ids are
/// DIDs, which use only `did:key:z...` characters -- none of the excluded
/// ones are ever legitimate here.
pub(crate) fn is_safe_service_id_for_path(service_id: &str) -> bool {
    !service_id.is_empty()
        && !service_id.contains('/')
        && !service_id.contains('\\')
        && !service_id.contains("..")
}

/// ADR-0018 §4: the substrate *validates* the declaration against the signed
/// artifact rather than deciding it -- `is_private` lives inside the
/// signature, so only the signer can set it. Returns the model `Visibility`
/// to record for this service.
pub(crate) fn validate_publication(
    service_id: &str,
    declared: Option<WitVisibility>,
    certificate: Option<&str>,
) -> Result<AppVisibility, String> {
    let v = match declared.unwrap_or(WitVisibility::Private) {
        WitVisibility::Public => AppVisibility::Public,
        WitVisibility::Internal => AppVisibility::Internal,
        WitVisibility::Private => AppVisibility::Private,
    };

    let v_str = v.as_str();

    match (v, certificate) {
        (AppVisibility::Private, None) => Ok(AppVisibility::Private),
        (AppVisibility::Private, Some(_)) => Err(format!(
            "service '{service_id}' declares visibility 'private' but a registry certificate was \
             supplied -- declare 'public' or 'internal', or deploy without the certificate"
        )),
        (AppVisibility::Public | AppVisibility::Internal, None) => Err(format!(
            "service '{service_id}' declares visibility '{v_str}' but no registry certificate was \
             supplied -- a record must be signed by the service's own key, which this substrate \
             does not hold"
        )),
        (AppVisibility::Public | AppVisibility::Internal, Some(json)) => {
            let signed = serde_json::from_str::<SignedEndpointInfo>(json).map_err(|e| {
                format!("registry certificate for '{service_id}' does not parse: {e}")
            })?;

            if signed.info.service_id != service_id {
                return Err(format!(
                    "registry certificate for '{service_id}' names service '{}' -- it would be \
                     rejected by the registry, which resolves the signing key from that field",
                    signed.info.service_id
                ));
            }

            let want_private = v == AppVisibility::Internal;
            if signed.info.is_private != want_private {
                return Err(format!(
                    "service '{service_id}' declares visibility '{v_str}', but its registry \
                     certificate carries is_private={}; the record is signed, so this can only be \
                     fixed by re-signing it",
                    signed.info.is_private
                ));
            }

            Ok(v)
        }
    }
}

/// `ServiceType` -> the string stored in `service_deploy_facts` and reported
/// on the wire. The inverse parse is `parse_service_type`, just below.
pub(crate) const fn service_type_str(t: AppServiceType) -> &'static str {
    match t {
        AppServiceType::Wasm => "wasm",
        AppServiceType::Container => "container",
        AppServiceType::Tcp => "tcp",
        AppServiceType::NativeHost => "nativehost",
    }
}

/// Validates one wire `dependency-binding` into `(LogicalServiceName,
/// TopologyEntry)`. Shared by the deploy path and `write_bindings` so the
/// two cannot validate differently -- every field is caller-supplied, and
/// `LogicalServiceName::new` *panics* on an empty name or one containing
/// '/'.
pub(crate) fn prepare_binding(
    binding: &DependencyBinding,
    app_instance_id: &str,
) -> Result<(LogicalServiceName, TopologyEntry), String> {
    // ADR-0021 §2: dependency resolution is intra-app only -- a
    // deploy (or a binding push) may bind dependencies for its own
    // declared app instance, never a different one. `DependencyBinding.
    // app_instance_id` is deliberately caller-supplied, ahead of the
    // cross-app `Bind` surface the WIT comment reserves it for ("equal to
    // the dependent's own app-instance-id today"); without this
    // comparison it goes unenforced, and one authorized writer could
    // silently overwrite the binding a *different* app instance's
    // services resolve.
    if binding.app_instance_id != app_instance_id {
        return Err(format!(
            "binding '{}' names app instance '{}', but this deploy's app context is '{}' -- a \
             deploy may only bind dependencies for its own app instance",
            binding.dependency_name, binding.app_instance_id, app_instance_id
        ));
    }
    let dependency_name = LogicalServiceName::try_new(&binding.dependency_name)
        .map_err(|e| format!("binding names an invalid dependency name: {e}"))?;
    let entry = TopologyEntry {
        mode: map_topology_mode(binding.mode),
        members: binding
            .members
            .iter()
            .map(AppServiceId::try_new)
            .collect::<result::Result<Vec<_>, _>>()
            .map_err(|e| {
                format!("binding '{}' names an invalid member DID: {e}", binding.dependency_name)
            })?,
        sharding_strategy: None, // D-A2-4
        epoch: TopologyEpoch(binding.epoch),
        cache_ttl: Duration::from_millis(binding.cache_ttl_ms),
        not_after: None,
    };
    Ok((dependency_name, entry))
}

/// `AppInstanceManagement` (the internal, storage-facing type) -> its wire
/// record. Kept as a free function rather than a `From` impl since the wire
/// type lives in a generated module neither type owns.
pub(crate) fn management_to_wire(m: &AppInstanceManagement) -> AppInstanceManagementWire {
    AppInstanceManagementWire {
        owner_did: m.owner_did.clone(),
        supervisor_did: m.supervisor_did.clone(),
        generation: m.generation,
    }
}

/// `BindingWriteOutcome` (the pure, `app_orchestration`-owned rule's
/// result) -> its wire variant. Same free-function shape as
/// `management_to_wire`, for the same reason.
pub(crate) const fn wire_binding_outcome(outcome: &BindingWriteOutcome) -> BindingWriteOutcomeWire {
    match outcome {
        BindingWriteOutcome::Applied => BindingWriteOutcomeWire::Applied,
        BindingWriteOutcome::NoOp => BindingWriteOutcomeWire::NoOp,
        BindingWriteOutcome::Stale(epoch) => BindingWriteOutcomeWire::Stale(epoch.0),
        BindingWriteOutcome::Conflict(epoch) => BindingWriteOutcomeWire::Conflict(epoch.0),
    }
}

pub(crate) fn parse_service_type(s: &str) -> Option<AppServiceType> {
    match s {
        "wasm" => Some(AppServiceType::Wasm),
        "container" => Some(AppServiceType::Container),
        "tcp" => Some(AppServiceType::Tcp),
        "nativehost" => Some(AppServiceType::NativeHost),
        _ => None,
    }
}

/// Wire `health-check` -> the app model's, so deploy-time validation can use
/// `HealthCheck::valid_for`/`kind_name` rather than restating the pairing
/// table on the wire type. The inverse of `syneroym_sdk::mapper`'s
/// `map_health_check`. Fallible, unlike that mapper direction: `interface`
/// is caller-supplied on this path (a wire deploy call, not a locally-parsed
/// manifest), so an empty name must be a deploy error, not a panic.
pub(crate) fn model_health_check(c: &WitHealthCheck) -> Result<HealthCheck, String> {
    let interface_name = |s: &str| {
        InterfaceName::try_new(s).map_err(|e| format!("invalid health check interface: {e}"))
    };
    Ok(match c {
        WitHealthCheck::TcpConnect(p) => HealthCheck::TcpConnect(TcpProbe {
            interface: interface_name(&p.interface_name)?,
            timeout_ms: p.timeout_ms,
        }),
        WitHealthCheck::HttpGet(p) => HealthCheck::HttpGet(HttpProbe {
            interface: interface_name(&p.interface_name)?,
            path: p.path.clone(),
            expect_status: p.expect_status,
            timeout_ms: p.timeout_ms,
        }),
        WitHealthCheck::Rpc(p) => HealthCheck::Rpc(RpcProbe {
            interface: interface_name(&p.interface_name)?,
            method: p.method.clone(),
            timeout_ms: p.timeout_ms,
        }),
    })
}

/// A deploy's `app_context`, validated but not yet written (A2, post-review
/// fix). Validation runs early -- a malformed or unauthorized binding is a
/// deploy failure, not a routing failure discovered later -- but the actual
/// registry/resolver write is deferred until every other fallible deploy
/// step has succeeded, so a deploy that goes on to fail never leaves a
/// binding installed. See `deploy_with_context` and `install_app_context`.
pub(crate) struct PreparedAppContext {
    pub(super) instance_id: AppInstanceId,
    pub(super) raw_instance_id: String,
    pub(super) raw_service_name: String,
    /// (`dependency_name` as sent on the wire, the validated
    /// `LogicalServiceName`, the resolved `TopologyEntry`) per binding.
    pub(super) bindings: Vec<(String, LogicalServiceName, TopologyEntry)>,
}

/// Resolves a manifest document to its content. `Inline` arrives with the
/// deploy call itself; `Path` is read from the substrate host's own
/// filesystem, under `deploy_docs`' traversal and size guards, on a blocking
/// thread since it touches the disk.
pub(crate) async fn resolve_document(
    source: &DocumentSource,
    field_name: &'static str,
) -> Result<String, String> {
    match source {
        DocumentSource::Inline(content) => {
            deploy_docs::check_inline_size(content, field_name)?;
            Ok(content.clone())
        }
        DocumentSource::Path(path) => {
            let path = PathBuf::from(path);
            task::spawn_blocking(move || deploy_docs::read_host_document(&path, field_name))
                .await
                .map_err(|e| format!("Failed to spawn blocking task: {e}"))?
        }
    }
}

/// Resolves an `asset-bundle.archive` field to raw bytes. `Binary`
/// is the only real case -- the SDK's mapper (`resolve_artifact_source`)
/// already turns a local path or an inlined hex artifact into `Binary` bytes
/// before the manifest ever reaches the wire. `Url` is a dead branch here,
/// exactly as it already is for the Wasm component's own `source` (nothing
/// fetches it); reviving it is a deferred item (deferred-backlog.md), so
/// it is rejected explicitly rather than silently accepted and ignored.
pub(crate) fn resolve_asset_archive(source: &ArtifactSource) -> Result<Vec<u8>, String> {
    match source {
        ArtifactSource::Binary(bytes) => Ok(bytes.clone()),
        ArtifactSource::Url(_) => Err("asset bundle archive via a URL artifact-source is not \
                                       supported; provide it as inline bytes"
            .to_string()),
    }
}

/// Deploy-time author warning: compares a deployed policy's
/// `definitions:` against the service's actual collections (its own tables
/// are the collection inventory -- a manifest declares no collection list of
/// its own). Warn-only in both directions, never a hard failure:
/// 1. a table with no matching `definitions:` entry is unfiltered today and
///    would be denied under `strict: true`;
/// 2. a `definitions:` entry whose `table` doesn't exist yet is expected for a
///    TCP/container service whose collections are created lazily on first use,
///    so it must not read as an error.
pub(crate) fn warn_on_policy_collection_mismatch(
    service_id: &str,
    policy: &Policy,
    collections: &[String],
) {
    let defined_tables: BTreeSet<&str> =
        policy.definitions.values().map(|d| d.table.as_str()).collect();
    for collection in collections {
        if !defined_tables.contains(collection.as_str()) {
            tracing::warn!(
                service_id,
                collection,
                "collection has no FDAE definition; it is unfiltered today and would be denied \
                 under `strict: true`"
            );
        }
    }
    for (type_name, def) in &policy.definitions {
        if !collections.iter().any(|c| c == &def.table) {
            tracing::warn!(
                service_id,
                definition = type_name.as_str(),
                table = def.table.as_str(),
                "policy defines a collection but no such collection exists yet -- expected for a \
                 TCP/container service whose collections are created lazily on first use"
            );
        }
    }
}

/// Author-time lint, in the same additive warn-only class as
/// `warn_on_policy_collection_mismatch`: flags a definition where an
/// unconditionally-public permission (`paths: []`, compiles to `1=1`) shares
/// a covering ability with a path-restricted sibling permission, and the two
/// aren't linked by `includes`. The compiler ORs every covering permission
/// together (`applicable_permissions`), so a caller holding a generic
/// ability-scoped capability -- not a named `app/<type>.<permission>` grant
/// -- that satisfies the restricted permission's ability is also admitted
/// through the public one, silently widening access past the restricted
/// permission's own `paths`. Sometimes intended (that's what `includes` is
/// for, to make it explicit); often a policy-authoring mistake, so it's
/// worth a loud warning even though nothing here justifies failing the
/// deploy.
pub(crate) fn warn_on_ambiguous_public_permission(service_id: &str, policy: &Policy) {
    for (type_name, def) in &policy.definitions {
        for (public_name, public_perm) in &def.permissions {
            if !public_perm.paths.is_empty() {
                continue;
            }
            for (restricted_name, restricted_perm) in &def.permissions {
                if public_name == restricted_name || restricted_perm.paths.is_empty() {
                    continue;
                }
                if public_perm.includes.contains(restricted_name)
                    || restricted_perm.includes.contains(public_name)
                {
                    continue;
                }
                let shares_covering_ability = public_perm.allows.iter().any(|a| {
                    restricted_perm.allows.iter().any(|b| {
                        let (a, b) = (Ability(a.clone()), Ability(b.clone()));
                        a.0 == b.0 || a.entails(&b) || b.entails(&a)
                    })
                });
                if shares_covering_ability {
                    tracing::warn!(
                        service_id,
                        definition = type_name.as_str(),
                        public_permission = public_name.as_str(),
                        restricted_permission = restricted_name.as_str(),
                        "an unconditionally public permission (paths: []) shares a covering \
                         ability with a path-restricted sibling permission and the two aren't \
                         linked by `includes` -- any capability admitted for the restricted \
                         permission is also admitted for the public one, silently granting \
                         unrestricted access unless callers only ever hold a named \
                         app/<type>.<permission> capability; link them with `includes` if this is \
                         intended"
                    );
                }
            }
        }
    }
}

/// Whether any permission in `policy` opts into the stage-4 after-step
/// (ADR-0017 §7, `authorize_rows: true`). Whole-policy, unlike
/// `syneroym_fdae::definition_has_abac`'s per-collection question -- the
/// deploy-time gate below needs to know before a single component/service
/// type is chosen, since a TCP/container service has no guest to call for
/// *any* collection.
pub(crate) fn policy_declares_stage4(policy: &Policy) -> bool {
    policy.definitions.values().any(|def| def.permissions.values().any(|p| p.authorize_rows))
}
