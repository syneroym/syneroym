use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::CallerContext;
use wasmtime::UpdateDeadline;

use super::*;
use crate::host_capabilities::{
    CallAll, CallAllLimits, HostState,
    tests::{
        test_blob_provider, test_messaging_context, test_service_proxy, test_streaming_context,
    },
};

#[test]
fn a_budget_that_was_never_armed_interrupts() {
    let budget = EpochBudget::new(Duration::from_secs(15));
    assert!(matches!(budget.on_deadline(Instant::now()), UpdateDeadline::Interrupt));
}

#[test]
fn zero_credit_interrupts_at_the_deadline_even_when_own_time_is_below_budget() {
    let mut budget = EpochBudget::new(Duration::from_secs(15));
    let start = Instant::now();
    budget.arm(start, 5); // 500 ms budget
    let now = start + Duration::from_millis(450);
    assert!(matches!(budget.on_deadline(now), UpdateDeadline::Interrupt));
}

#[test]
fn credit_extends_by_the_budget_left_rounded_up() {
    let mut budget = EpochBudget::new(Duration::from_secs(15));
    let start = Instant::now();
    budget.arm(start, 5); // 500 ms budget
    budget.credit(WaitCredit::for_test(Duration::from_millis(1000)));

    // Elapsed 1200 ms, credited 1000 ms -> own time 200 ms.
    // Remaining budget = 300 ms -> 3 ticks of 100 ms.
    let now_1200 = start + Duration::from_millis(1200);
    assert!(matches!(budget.on_deadline(now_1200), UpdateDeadline::Continue(3)));

    // Elapsed 1500 ms, credited 1000 ms -> own time 500 ms.
    // Own time equals budget -> Interrupt.
    let now_1500 = start + Duration::from_millis(1500);
    assert!(matches!(budget.on_deadline(now_1500), UpdateDeadline::Interrupt));
}

#[test]
fn credit_is_capped() {
    let mut budget = EpochBudget::new(Duration::from_millis(1000));
    let start = Instant::now();
    budget.arm(start, 5);
    budget.credit(WaitCredit::for_test(Duration::from_millis(1500)));
    assert_eq!(budget.allowance(), Duration::ZERO);
    assert_eq!(budget.credited, Duration::from_millis(1000));
}

#[test]
fn allowance_is_the_cap_minus_the_credit() {
    let mut budget = EpochBudget::new(Duration::from_millis(1000));
    let start = Instant::now();
    budget.arm(start, 5);
    assert_eq!(budget.allowance(), Duration::from_millis(1000));
    budget.credit(WaitCredit::for_test(Duration::from_millis(400)));
    assert_eq!(budget.allowance(), Duration::from_millis(600));
}

#[test]
fn arm_resets_credit_and_start() {
    let mut budget = EpochBudget::new(Duration::from_millis(1000));
    let start1 = Instant::now();
    budget.arm(start1, 5);
    budget.credit(WaitCredit::for_test(Duration::from_millis(400)));
    assert_eq!(budget.allowance(), Duration::from_millis(600));

    let start2 = Instant::now() + Duration::from_millis(100);
    budget.arm(start2, 10);
    assert_eq!(budget.started, start2);
    assert_eq!(budget.budget, Duration::from_millis(1000));
    assert_eq!(budget.credited, Duration::ZERO);
    assert_eq!(budget.allowance(), Duration::from_millis(1000));
}

fn collect_production_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            collect_production_source_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if path.file_name().is_some_and(|name| name == "tests.rs") {
                continue;
            }
            files.push(path);
        }
    }
}

#[test]
fn no_production_code_sets_an_epoch_deadline_itself() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    let mut files = Vec::new();
    collect_production_source_files(&src_dir, &mut files);

    let mut set_epoch_files = Vec::new();
    let mut arm_epoch_files = Vec::new();

    for file in files {
        let content = fs::read_to_string(&file).expect("read file");
        let set_count = content.matches("set_epoch_deadline(").count();
        if set_count > 0 {
            set_epoch_files.push((file.clone(), set_count));
        }
        let arm_count = content.matches("arm_epoch_deadline(").count();
        if arm_count > 0 {
            arm_epoch_files.push((file.clone(), arm_count));
        }
    }

    assert_eq!(
        set_epoch_files.len(),
        1,
        "set_epoch_deadline( should appear only in epoch_budget.rs, found in: {:?}",
        set_epoch_files
    );
    assert!(
        set_epoch_files[0].0.ends_with("epoch_budget.rs"),
        "expected epoch_budget.rs, found {:?}",
        set_epoch_files[0].0
    );
    assert_eq!(set_epoch_files[0].1, 1, "expected exactly 1 call in epoch_budget.rs");

    assert_eq!(
        arm_epoch_files.len(),
        3,
        "arm_epoch_deadline( must appear only in epoch_budget.rs, engine.rs and stream.rs: {:?}",
        arm_epoch_files
    );
    let mut matched_names: Vec<String> = arm_epoch_files
        .iter()
        .map(|(path, count)| {
            assert_eq!(*count, 1, "expected count 1 in {:?}", path);
            path.file_name().unwrap().to_string_lossy().into_owned()
        })
        .collect();
    matched_names.sort();
    assert_eq!(matched_names, vec!["engine.rs", "epoch_budget.rs", "stream.rs"]);
}

fn test_host_state() -> HostState {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    HostState::new(
        "test-component".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        CallerContext::service_system("test-component"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
}

#[test]
fn a_host_state_that_was_never_armed_allows_a_full_batch() {
    let host_state = test_host_state();
    assert_eq!(host_state.epoch.allowance(), CallAllLimits::default().total_timeout);
}

#[test]
fn with_call_all_sets_the_credit_cap() {
    let host_state = test_host_state();
    let custom_limits =
        CallAllLimits { total_timeout: Duration::from_secs(7), ..CallAllLimits::default() };
    let configured_host_state = host_state.with_call_all(CallAll::new(custom_limits, 4));
    assert_eq!(configured_host_state.epoch.allowance(), Duration::from_secs(7));
}
