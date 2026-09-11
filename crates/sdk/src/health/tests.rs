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
        member_index: 0,
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
        binding_epochs: Vec::new(),
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

/// `ServiceHealth.binding_epochs` must actually carry
/// what `ServiceStatus.binding_epochs` reported -- the field the
/// supervisor's convergence check depends on, dropped silently before
/// this fix.
#[tokio::test]
async fn poll_once_carries_each_services_binding_epochs_through_to_service_health() {
    let mut status = service_status("did:key:svc", InstancePhase::Running, ProbeStatus::Passing);
    status.binding_epochs = vec![("backend".to_string(), 3)];
    let status = SubstrateStatus { node: None, checked_at: 0, services: vec![status] };
    let targets = BTreeMap::from([(
        "did:key:b".to_string(),
        target("did:key:b", Arc::new(FakeQuery::ok(status))),
    )]);
    let expected = vec![expected("frontend", "did:key:svc", "did:key:b")];

    let report = poll_once(&targets, &expected).await;
    assert_eq!(report.services[0].binding_epochs, vec![("backend".to_string(), 3)]);
}

/// The arm that would otherwise panic or fabricate: an unreachable
/// substrate has no answer to read a binding epoch from, so the field
/// must read empty rather than stale or synthesized.
#[tokio::test]
async fn poll_once_reports_empty_binding_epochs_for_an_unreachable_substrate() {
    let targets = BTreeMap::from([(
        "did:key:b".to_string(),
        target("did:key:b", Arc::new(FakeQuery::err("connection refused"))),
    )]);
    let expected = vec![expected("backend", "did:key:svc", "did:key:b")];

    let report = poll_once(&targets, &expected).await;
    assert!(report.services[0].binding_epochs.is_empty());
}

#[tokio::test]
async fn a_service_with_no_completed_placement_reports_not_deployed() {
    let targets = BTreeMap::new();
    let expected = vec![ExpectedService {
        logical_ref: l_ref("backend"),
        service_id: String::new(),
        substrate_did: String::new(),
        member_index: 0,
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
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    record_report(&alerts, &instance_id, &not_running, 1000, &[], CertAlertPolicy::Reminder)
        .unwrap();
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
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    record_report(&alerts, &instance_id, &probe_failing, 1001, &[], CertAlertPolicy::Reminder)
        .unwrap();
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
            fault: Some(SubstrateFault::Unreachable("unreachable".to_string())),
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
                binding_epochs: Vec::new(),
                member_index: 0,
            },
            ServiceHealth {
                logical_ref: l_ref("backend2"),
                service_id: "did:key:svc2".to_string(),
                alias: None,
                substrate_did: "did:key:b".to_string(),
                signal: Signal::SubstrateUnreachable("unreachable".to_string()),
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
                binding_epochs: Vec::new(),
                member_index: 0,
            },
        ],
    };
    let opened =
        record_report(&alerts, &instance_id, &report, 1000, &[], CertAlertPolicy::Reminder)
            .unwrap();
    assert_eq!(opened.iter().filter(|(k, _)| *k == AlertKind::SubstrateUnreachable).count(), 1);
    assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);
}

/// An untargeted substrate (the caller built no `HealthTarget` for
/// it -- e.g. an inventory entry the caller's own config dropped) is a
/// configuration gap, not a live outage, and must not raise the same
/// alert a genuinely unreachable substrate does.
#[tokio::test]
async fn record_report_does_not_raise_substrate_unreachable_for_an_untargeted_substrate() {
    let alerts = AlertStore::open_in_memory().unwrap();
    let instance_id = AppInstanceId::new("inst-1");

    let report = HealthReport {
        substrates: vec![SubstrateHealth {
            alias: None,
            substrate_did: "did:key:b".to_string(),
            node: None,
            fault: Some(SubstrateFault::NoTargetBuilt(
                "no health target built for this substrate".to_string(),
            )),
        }],
        services: vec![ServiceHealth {
            logical_ref: l_ref("backend"),
            service_id: "did:key:svc1".to_string(),
            alias: None,
            substrate_did: "did:key:b".to_string(),
            signal: Signal::Unknown("no health target built for this substrate".to_string()),
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    let opened =
        record_report(&alerts, &instance_id, &report, 1000, &[], CertAlertPolicy::Reminder)
            .unwrap();
    assert!(opened.is_empty(), "{opened:?}");
    assert!(alerts.active(&instance_id).unwrap().is_empty());
}

/// Once a service leaves the sweep entirely (removed from the
/// manifest, or `app forget`), no later report ever mentions it again --
/// so its last-raised row must be cleared the moment it disappears, not
/// left active forever.
#[tokio::test]
async fn record_report_clears_an_alert_for_a_service_no_longer_in_the_report() {
    let alerts = AlertStore::open_in_memory().unwrap();
    let instance_id = AppInstanceId::new("inst-1");

    let failing = HealthReport {
        substrates: vec![],
        services: vec![ServiceHealth {
            logical_ref: l_ref("backend"),
            service_id: "did:key:svc".to_string(),
            alias: None,
            substrate_did: "did:key:b".to_string(),
            signal: Signal::ProbeFailing("bad".to_string()),
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    record_report(&alerts, &instance_id, &failing, 1000, &[], CertAlertPolicy::Reminder).unwrap();
    assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);

    // The next sweep no longer names "backend" at all -- removed from
    // the manifest, or forgotten.
    let empty = HealthReport { substrates: vec![], services: vec![] };
    record_report(&alerts, &instance_id, &empty, 1001, &[], CertAlertPolicy::Reminder).unwrap();
    assert!(alerts.active(&instance_id).unwrap().is_empty());
    assert_eq!(alerts.all(&instance_id).unwrap().len(), 1, "the row must still be readable");
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
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    record_report(&alerts, &instance_id, &report, now, &[], CertAlertPolicy::Reminder).unwrap();
    assert!(alerts.active(&instance_id).unwrap().is_empty());
}

/// An already-expired certificate must not read as a near-expiry
/// reminder -- it is a current outage under the attended posture,
/// and the two alert kinds must never both be active for the same
/// service at once.
#[tokio::test]
async fn record_report_raises_certificate_expired_not_near_expiry_once_past_the_window() {
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
            instance_certificate_issued_at: Some(now - 25 * 3600),
            instance_certificate_expires_at: Some(now - 3600),
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    let opened =
        record_report(&alerts, &instance_id, &report, now, &[], CertAlertPolicy::Reminder).unwrap();
    assert_eq!(opened, vec![(AlertKind::CertificateExpired, "inst-1/backend#0".to_string())]);
    let active = alerts.active(&instance_id).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].kind, AlertKind::CertificateExpired);
}

/// `syneroym-app-supervisor`'s resident loop is the
/// sole producer of `CertificateNearExpiry`/`CertificateExpired` for
/// its own instances. Before `CertAlertPolicy` existed, this same
/// near-expiry window unconditionally raised (and published) one of
/// these on every renewal cycle, whether or not the renewal that ran
/// moments later in the same pass actually succeeded -- and made a
/// genuinely stalled renewal's own raise a silent no-op against the
/// row this call had already opened. `ManagedElsewhere` must raise
/// neither kind, for a near-expiry certificate or an expired one,
/// leaving the caller as the only producer.
#[tokio::test]
async fn managed_elsewhere_raises_neither_cert_alert_kind() {
    let alerts = AlertStore::open_in_memory().unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    let now = 1_700_000_000u64;

    let near_expiry = HealthReport {
        substrates: vec![],
        services: vec![ServiceHealth {
            logical_ref: l_ref("backend"),
            service_id: "did:key:svc".to_string(),
            alias: None,
            substrate_did: "did:key:b".to_string(),
            signal: Signal::Healthy,
            instance_certificate_issued_at: Some(now - 21_600),
            instance_certificate_expires_at: Some(now + 1_440),
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    let opened = record_report(
        &alerts,
        &instance_id,
        &near_expiry,
        now,
        &[],
        CertAlertPolicy::ManagedElsewhere,
    )
    .unwrap();
    assert!(opened.is_empty(), "{opened:?}");
    assert!(alerts.active(&instance_id).unwrap().is_empty());

    let expired = HealthReport {
        substrates: vec![],
        services: vec![ServiceHealth {
            instance_certificate_issued_at: Some(now - 25 * 3600),
            instance_certificate_expires_at: Some(now - 3600),
            ..near_expiry.services[0].clone()
        }],
    };
    let opened =
        record_report(&alerts, &instance_id, &expired, now, &[], CertAlertPolicy::ManagedElsewhere)
            .unwrap();
    assert!(opened.is_empty(), "{opened:?}");
    assert!(alerts.active(&instance_id).unwrap().is_empty());
}

/// The other half: `ManagedElsewhere` must not clear either kind
/// either, since the caller -- not this sweep -- decides when a
/// stalled renewal's alert is settled.
#[tokio::test]
async fn managed_elsewhere_does_not_clear_a_cert_alert_the_caller_raised() {
    let alerts = AlertStore::open_in_memory().unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    let now = 1_700_000_000u64;
    alerts
        .raise(
            &instance_id,
            Some("inst-1/backend#0"),
            None,
            "did:key:b",
            AlertKind::CertificateNearExpiry,
            "stalled, raised by the caller",
        )
        .unwrap();

    let fresh = HealthReport {
        substrates: vec![],
        services: vec![ServiceHealth {
            logical_ref: l_ref("backend"),
            service_id: "did:key:svc".to_string(),
            alias: None,
            substrate_did: "did:key:b".to_string(),
            signal: Signal::Healthy,
            instance_certificate_issued_at: Some(now),
            instance_certificate_expires_at: Some(now + 14_400),
            binding_epochs: Vec::new(),
            member_index: 0,
        }],
    };
    record_report(&alerts, &instance_id, &fresh, now, &[], CertAlertPolicy::ManagedElsewhere)
        .unwrap();

    assert_eq!(
        alerts.active(&instance_id).unwrap().len(),
        1,
        "only the caller's own clearing rule may settle this alert"
    );
}
