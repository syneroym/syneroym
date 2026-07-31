//! Polling the health of an app instance's services across the substrates
//! they are placed on (M05A Slice A4), read-only.
//!
//! `StatusQuery` is the read-side twin of `deploy::PlanApplier`, and lives
//! here for the same two reasons: the two-node e2e can drive it (`sdk` is a
//! dev-dependency of `crates/substrate`, `roymctl` is a binary that cannot be
//! linked from a test), and A5's reconcile loop calls this same function
//! rather than growing a second poller beside it.

use std::{collections::BTreeMap, fmt, sync::Arc};

use anyhow::Result;
use syneroym_app_orchestration::{
    AlertKind, AlertStore,
    models::{AppInstanceId, LogicalServiceRef, SubstrateAlias},
};
use syneroym_identity::delegation::is_near_expiry_parts;

use crate::{InstancePhase, NodeFacts, ProbeStatus, SubstrateStatus, SyneroymClient};

#[async_trait::async_trait]
pub trait StatusQuery: fmt::Debug + Send + Sync {
    async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus, String>;
}

#[async_trait::async_trait]
impl StatusQuery for SyneroymClient {
    async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus, String> {
        self.status(service_ids).await.map_err(|e| e.to_string())
    }
}

/// A substrate to poll. Mirrors `deploy::DeployTarget` field for field.
#[derive(Debug, Clone)]
pub struct HealthTarget {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub query: Arc<dyn StatusQuery>,
}

/// One service the sweep expects to find, already resolved to the DID it was
/// actually deployed under (D-A4-11 -- the caller resolves it, because the
/// member-master identity files live under `roymctl`'s `--dir`, not here).
/// An empty `substrate_did` means the journal records no completed placement.
#[derive(Debug, Clone)]
pub struct ExpectedService {
    pub logical_ref: LogicalServiceRef,
    pub service_id: String,
    pub substrate_did: String,
}

/// The three signals `task.md` requires stay distinct, plus the two states
/// that are neither healthy nor a fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Healthy,
    /// The substrate did not answer. Never inferred from a service-level
    /// symptom (D-A4-13).
    SubstrateUnreachable(String),
    InstanceNotRunning(String),
    ProbeFailing(String),
    /// The substrate answered but cannot tell -- a `tcp` service that
    /// declared no probe, a service with no recorded type, or a caller
    /// without the grant. Not healthy, and not a fault (D-A4-19).
    Unknown(String),
    /// The journal has no completed placement for this service.
    NotDeployed,
}

impl Signal {
    /// Whether this is one of the three faults `task.md` names. `Unknown`
    /// and `NotDeployed` are deliberately excluded: "I cannot tell" must not
    /// drive a non-zero exit or an alert.
    #[must_use]
    pub const fn is_fault(&self) -> bool {
        matches!(
            self,
            Self::SubstrateUnreachable(_) | Self::InstanceNotRunning(_) | Self::ProbeFailing(_)
        )
    }
}

#[derive(Debug, Clone)]
pub struct ServiceHealth {
    pub logical_ref: LogicalServiceRef,
    pub service_id: String,
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub signal: Signal,
    pub instance_certificate_issued_at: Option<u64>,
    pub instance_certificate_expires_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SubstrateHealth {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    /// `None` when the substrate did not answer, or when this caller holds
    /// no node-wide `orchestrator/status` (D-A4-18) -- `error` distinguishes
    /// the two.
    pub node: Option<NodeFacts>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct HealthReport {
    pub substrates: Vec<SubstrateHealth>,
    pub services: Vec<ServiceHealth>,
}

impl HealthReport {
    /// Services reporting one of the three faults. Drives the exit code
    /// (D-A4-19).
    #[must_use]
    pub fn faults(&self) -> Vec<&ServiceHealth> {
        self.services.iter().filter(|s| s.signal.is_fault()).collect()
    }

    /// Services the sweep could not decide. Reported, never fatal unless the
    /// caller asked for `--strict`.
    #[must_use]
    pub fn unknowns(&self) -> Vec<&ServiceHealth> {
        self.services
            .iter()
            .filter(|s| matches!(s.signal, Signal::Unknown(_) | Signal::NotDeployed))
            .collect()
    }

    /// No faults and nothing unknown.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.faults().is_empty() && self.unknowns().is_empty()
    }
}

/// Polls every distinct substrate `expected` names, and folds each answer
/// into a per-service `Signal`.
pub async fn poll_once(
    targets: &BTreeMap<String, HealthTarget>,
    expected: &[ExpectedService],
) -> HealthReport {
    let mut report = HealthReport::default();

    let mut by_substrate: BTreeMap<&str, Vec<&ExpectedService>> = BTreeMap::new();
    for svc in expected {
        if svc.substrate_did.is_empty() {
            continue;
        }
        by_substrate.entry(svc.substrate_did.as_str()).or_default().push(svc);
    }

    for (did, services) in by_substrate {
        let Some(target) = targets.get(did) else {
            // A placement whose substrate the caller built no target for --
            // an inventory that no longer lists the alias, say. Reported,
            // not skipped: silently dropping it would show a healthy sweep
            // for an app that is partly unwatched.
            let msg = "no health target built for this substrate".to_string();
            report.substrates.push(SubstrateHealth {
                alias: None,
                substrate_did: did.to_string(),
                node: None,
                error: Some(msg.clone()),
            });
            for s in services {
                report.services.push(ServiceHealth {
                    logical_ref: s.logical_ref.clone(),
                    service_id: s.service_id.clone(),
                    alias: None,
                    substrate_did: did.to_string(),
                    signal: Signal::Unknown(msg.clone()),
                    instance_certificate_issued_at: None,
                    instance_certificate_expires_at: None,
                });
            }
            continue;
        };

        let ids: Vec<String> = services.iter().map(|s| s.service_id.clone()).collect();
        match target.query.status(ids).await {
            Err(e) => {
                // D-A4-13: one substrate-level fault, no per-service alerts.
                report.substrates.push(SubstrateHealth {
                    alias: target.alias.clone(),
                    substrate_did: did.to_string(),
                    node: None,
                    error: Some(e.clone()),
                });
                for s in services {
                    report.services.push(ServiceHealth {
                        logical_ref: s.logical_ref.clone(),
                        service_id: s.service_id.clone(),
                        alias: target.alias.clone(),
                        substrate_did: did.to_string(),
                        signal: Signal::SubstrateUnreachable(e.clone()),
                        instance_certificate_issued_at: None,
                        instance_certificate_expires_at: None,
                    });
                }
            }
            Ok(status) => {
                report.substrates.push(SubstrateHealth {
                    alias: target.alias.clone(),
                    substrate_did: did.to_string(),
                    node: status.node,
                    error: None,
                });
                let by_id: BTreeMap<&str, _> =
                    status.services.iter().map(|s| (s.service_id.as_str(), s)).collect();
                for s in services {
                    let Some(st) = by_id.get(s.service_id.as_str()) else {
                        report.services.push(ServiceHealth {
                            logical_ref: s.logical_ref.clone(),
                            service_id: s.service_id.clone(),
                            alias: target.alias.clone(),
                            substrate_did: did.to_string(),
                            signal: Signal::Unknown(
                                "the substrate returned no status for this id".to_string(),
                            ),
                            instance_certificate_issued_at: None,
                            instance_certificate_expires_at: None,
                        });
                        continue;
                    };
                    let signal = match (&st.phase, &st.probe) {
                        (InstancePhase::NotRunning(r), _) => Signal::InstanceNotRunning(r.clone()),
                        (InstancePhase::NotFound, _) => Signal::InstanceNotRunning(
                            "this substrate has no endpoints for the id".to_string(),
                        ),
                        (InstancePhase::Unauthorized, _) => Signal::Unknown(
                            "caller holds no orchestrator/status grant for this service"
                                .to_string(),
                        ),
                        // A failing probe is a fault whether or not the
                        // substrate could determine a phase -- for a `tcp`
                        // service it is the only signal there is (D-A4-7).
                        (
                            InstancePhase::Running | InstancePhase::Unknown(_),
                            ProbeStatus::Failing(r),
                        ) => Signal::ProbeFailing(r.clone()),
                        (InstancePhase::Running, _) => Signal::Healthy,
                        (InstancePhase::Unknown(_), ProbeStatus::Passing) => Signal::Healthy,
                        (InstancePhase::Unknown(r), ProbeStatus::NotDeclared) => {
                            Signal::Unknown(r.clone())
                        }
                    };
                    report.services.push(ServiceHealth {
                        logical_ref: s.logical_ref.clone(),
                        service_id: s.service_id.clone(),
                        alias: target.alias.clone(),
                        substrate_did: did.to_string(),
                        signal,
                        instance_certificate_issued_at: st.instance_certificate_issued_at,
                        instance_certificate_expires_at: st.instance_certificate_expires_at,
                    });
                }
            }
        }
    }

    for s in expected.iter().filter(|s| s.substrate_did.is_empty()) {
        report.services.push(ServiceHealth {
            logical_ref: s.logical_ref.clone(),
            service_id: s.service_id.clone(),
            alias: None,
            substrate_did: String::new(),
            signal: Signal::NotDeployed,
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
        });
    }

    report.services.sort_by(|a, b| a.logical_ref.cmp(&b.logical_ref));
    report
}

/// Folds a report into the alert store: raises what is now failing, clears
/// what is no longer. Returns the alerts this sweep *opened*, so a caller
/// prints transitions rather than re-printing the standing set every time.
pub fn record_report(
    alerts: &AlertStore,
    instance_id: &AppInstanceId,
    report: &HealthReport,
    now: u64,
) -> Result<Vec<(AlertKind, String)>> {
    let mut opened = Vec::new();

    for sub in &report.substrates {
        match &sub.error {
            Some(e) => {
                if alerts.raise(
                    instance_id,
                    None,
                    sub.alias.as_ref().map(SubstrateAlias::as_str),
                    &sub.substrate_did,
                    AlertKind::SubstrateUnreachable,
                    e,
                )? {
                    opened.push((AlertKind::SubstrateUnreachable, sub.substrate_did.clone()));
                }
            }
            None => {
                alerts.clear(
                    instance_id,
                    None,
                    &sub.substrate_did,
                    AlertKind::SubstrateUnreachable,
                )?;
            }
        }
    }

    for svc in &report.services {
        let l_ref = svc.logical_ref.to_string();
        // Exactly one of the two service-level kinds can be active at a
        // time; the other is cleared on every pass, so a service that moves
        // from "not running" to "probe failing" does not leave a stale
        // alert. `SubstrateUnreachable` is deliberately not re-raised per
        // service -- it was already raised once above (D-A4-13).
        let active = match &svc.signal {
            Signal::InstanceNotRunning(r) => Some((AlertKind::InstanceNotRunning, r.clone())),
            Signal::ProbeFailing(r) => Some((AlertKind::ProbeFailing, r.clone())),
            _ => None,
        };
        for kind in [AlertKind::InstanceNotRunning, AlertKind::ProbeFailing] {
            match &active {
                Some((k, detail)) if *k == kind => {
                    if alerts.raise(
                        instance_id,
                        Some(&l_ref),
                        svc.alias.as_ref().map(SubstrateAlias::as_str),
                        &svc.substrate_did,
                        kind,
                        detail,
                    )? {
                        opened.push((kind, l_ref.clone()));
                    }
                }
                _ => {
                    alerts.clear(instance_id, Some(&l_ref), &svc.substrate_did, kind)?;
                }
            }
        }

        // D-A4-16: the same relative rule the substrate's own heartbeat
        // sweep uses.
        let near = match (svc.instance_certificate_issued_at, svc.instance_certificate_expires_at) {
            (Some(issued), Some(expires)) => is_near_expiry_parts(issued, expires, now),
            _ => false,
        };
        if near {
            let detail = format!(
                "instance certificate expires at {}; renew with `roymctl identity \
                 certify-instance`",
                svc.instance_certificate_expires_at.unwrap_or(0)
            );
            if alerts.raise(
                instance_id,
                Some(&l_ref),
                svc.alias.as_ref().map(SubstrateAlias::as_str),
                &svc.substrate_did,
                AlertKind::CertificateNearExpiry,
                &detail,
            )? {
                opened.push((AlertKind::CertificateNearExpiry, l_ref.clone()));
            }
        } else {
            alerts.clear(
                instance_id,
                Some(&l_ref),
                &svc.substrate_did,
                AlertKind::CertificateNearExpiry,
            )?;
        }
    }

    Ok(opened)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use syneroym_app_orchestration::models::{AppInstanceId, LogicalServiceName};

    use super::*;
    use crate::ServiceStatus;

    #[derive(Debug, Default)]
    struct FakeQuery {
        result: Mutex<Option<Result<SubstrateStatus, String>>>,
    }

    impl FakeQuery {
        fn ok(status: SubstrateStatus) -> Self {
            Self { result: Mutex::new(Some(Ok(status))) }
        }
        fn err(msg: &str) -> Self {
            Self { result: Mutex::new(Some(Err(msg.to_string()))) }
        }
    }

    #[async_trait::async_trait]
    impl StatusQuery for FakeQuery {
        async fn status(&self, _service_ids: Vec<String>) -> Result<SubstrateStatus, String> {
            self.result.lock().unwrap().take().expect("status called more than once")
        }
    }

    fn l_ref(name: &str) -> LogicalServiceRef {
        LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new(name),
        }
    }

    fn expected(name: &str, service_id: &str, did: &str) -> ExpectedService {
        ExpectedService {
            logical_ref: l_ref(name),
            service_id: service_id.to_string(),
            substrate_did: did.to_string(),
        }
    }

    fn service_status(id: &str, phase: InstancePhase, probe: ProbeStatus) -> ServiceStatus {
        ServiceStatus {
            service_id: id.to_string(),
            service_type: Some("tcp".to_string()),
            endpoint_type: "tcp".to_string(),
            app_instance_id: None,
            service_name: None,
            phase,
            probe,
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            probe_checked_at: None,
        }
    }

    fn target(did: &str, query: Arc<dyn StatusQuery>) -> HealthTarget {
        HealthTarget { alias: None, substrate_did: did.to_string(), query }
    }

    #[tokio::test]
    async fn an_unreachable_substrate_marks_its_services_unreachable_and_not_not_running() {
        let targets = BTreeMap::from([(
            "did:key:b".to_string(),
            target("did:key:b", Arc::new(FakeQuery::err("connection refused"))),
        )]);
        let expected = vec![expected("backend", "did:key:svc", "did:key:b")];

        let report = poll_once(&targets, &expected).await;
        assert_eq!(report.services.len(), 1);
        assert_eq!(
            report.services[0].signal,
            Signal::SubstrateUnreachable("connection refused".to_string())
        );
        assert!(report.services[0].signal.is_fault());
    }

    #[tokio::test]
    async fn a_failing_probe_on_an_unknown_phase_reports_probe_failing() {
        let status = SubstrateStatus {
            node: None,
            checked_at: 0,
            services: vec![service_status(
                "did:key:svc",
                InstancePhase::Unknown("tcp runs outside this substrate".to_string()),
                ProbeStatus::Failing("connect refused".to_string()),
            )],
        };
        let targets = BTreeMap::from([(
            "did:key:b".to_string(),
            target("did:key:b", Arc::new(FakeQuery::ok(status))),
        )]);
        let expected = vec![expected("backend", "did:key:svc", "did:key:b")];

        let report = poll_once(&targets, &expected).await;
        assert_eq!(report.services[0].signal, Signal::ProbeFailing("connect refused".to_string()));
    }

    #[tokio::test]
    async fn a_stopped_instance_reports_instance_not_running_and_does_not_report_probe_failing() {
        let status = SubstrateStatus {
            node: None,
            checked_at: 0,
            services: vec![service_status(
                "did:key:svc",
                InstancePhase::NotRunning("no compiled component is loaded".to_string()),
                ProbeStatus::NotDeclared,
            )],
        };
        let targets = BTreeMap::from([(
            "did:key:b".to_string(),
            target("did:key:b", Arc::new(FakeQuery::ok(status))),
        )]);
        let expected = vec![expected("backend", "did:key:svc", "did:key:b")];

        let report = poll_once(&targets, &expected).await;
        assert!(matches!(report.services[0].signal, Signal::InstanceNotRunning(_)));
    }

    #[tokio::test]
    async fn an_undetermined_service_is_not_a_fault() {
        let status = SubstrateStatus {
            node: None,
            checked_at: 0,
            services: vec![service_status(
                "did:key:svc",
                InstancePhase::Unknown("tcp services run outside this substrate".to_string()),
                ProbeStatus::NotDeclared,
            )],
        };
        let targets = BTreeMap::from([(
            "did:key:b".to_string(),
            target("did:key:b", Arc::new(FakeQuery::ok(status))),
        )]);
        let expected = vec![expected("backend", "did:key:svc", "did:key:b")];

        let report = poll_once(&targets, &expected).await;
        assert!(report.faults().is_empty());
        assert_eq!(report.unknowns().len(), 1);
        assert!(!report.is_healthy());
    }

    #[tokio::test]
    async fn a_service_with_no_completed_placement_reports_not_deployed() {
        let targets = BTreeMap::new();
        let expected = vec![ExpectedService {
            logical_ref: l_ref("backend"),
            service_id: String::new(),
            substrate_did: String::new(),
        }];

        let report = poll_once(&targets, &expected).await;
        assert_eq!(report.services.len(), 1);
        assert_eq!(report.services[0].signal, Signal::NotDeployed);
    }

    #[tokio::test]
    async fn record_report_clears_a_service_level_alert_when_the_signal_changes_kind() {
        let alerts = AlertStore::open_in_memory().unwrap();
        let instance_id = AppInstanceId::new("inst-1");

        let not_running = HealthReport {
            substrates: vec![],
            services: vec![ServiceHealth {
                logical_ref: l_ref("backend"),
                service_id: "did:key:svc".to_string(),
                alias: None,
                substrate_did: "did:key:b".to_string(),
                signal: Signal::InstanceNotRunning("down".to_string()),
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
            }],
        };
        record_report(&alerts, &instance_id, &not_running, 1000).unwrap();
        assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);
        assert_eq!(alerts.active(&instance_id).unwrap()[0].kind, AlertKind::InstanceNotRunning);

        let probe_failing = HealthReport {
            substrates: vec![],
            services: vec![ServiceHealth {
                logical_ref: l_ref("backend"),
                service_id: "did:key:svc".to_string(),
                alias: None,
                substrate_did: "did:key:b".to_string(),
                signal: Signal::ProbeFailing("bad".to_string()),
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
            }],
        };
        record_report(&alerts, &instance_id, &probe_failing, 1001).unwrap();
        let active = alerts.active(&instance_id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].kind, AlertKind::ProbeFailing);
    }

    #[tokio::test]
    async fn record_report_raises_one_substrate_alert_not_one_per_service() {
        let alerts = AlertStore::open_in_memory().unwrap();
        let instance_id = AppInstanceId::new("inst-1");

        let report = HealthReport {
            substrates: vec![SubstrateHealth {
                alias: None,
                substrate_did: "did:key:b".to_string(),
                node: None,
                error: Some("unreachable".to_string()),
            }],
            services: vec![
                ServiceHealth {
                    logical_ref: l_ref("backend"),
                    service_id: "did:key:svc1".to_string(),
                    alias: None,
                    substrate_did: "did:key:b".to_string(),
                    signal: Signal::SubstrateUnreachable("unreachable".to_string()),
                    instance_certificate_issued_at: None,
                    instance_certificate_expires_at: None,
                },
                ServiceHealth {
                    logical_ref: l_ref("backend2"),
                    service_id: "did:key:svc2".to_string(),
                    alias: None,
                    substrate_did: "did:key:b".to_string(),
                    signal: Signal::SubstrateUnreachable("unreachable".to_string()),
                    instance_certificate_issued_at: None,
                    instance_certificate_expires_at: None,
                },
            ],
        };
        let opened = record_report(&alerts, &instance_id, &report, 1000).unwrap();
        assert_eq!(opened.iter().filter(|(k, _)| *k == AlertKind::SubstrateUnreachable).count(), 1);
        assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn record_report_does_not_raise_near_expiry_for_a_freshly_issued_certificate() {
        let alerts = AlertStore::open_in_memory().unwrap();
        let instance_id = AppInstanceId::new("inst-1");
        let now = 1_700_000_000u64;

        let report = HealthReport {
            substrates: vec![],
            services: vec![ServiceHealth {
                logical_ref: l_ref("backend"),
                service_id: "did:key:svc".to_string(),
                alias: None,
                substrate_did: "did:key:b".to_string(),
                signal: Signal::Healthy,
                instance_certificate_issued_at: Some(now),
                instance_certificate_expires_at: Some(now + 24 * 3600),
            }],
        };
        record_report(&alerts, &instance_id, &report, now).unwrap();
        assert!(alerts.active(&instance_id).unwrap().is_empty());
    }
}
