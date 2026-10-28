#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Pins the Wasmtime epoch-callback behaviour that the batched outbound call
//! relies on.
//!
//! These tests verify that host calls can be credited against the guest epoch
//! deadline, so a guest waiting on slow outbound calls is not interrupted,
//! while a guest looping in its own code is still interrupted on time.
//!
//! Multi-thread runtime note: with `#[tokio::test(flavor = "multi_thread")]`,
//! the guest execution runs on the test's `block_on` thread, not a worker.
//! The epoch ticker runs on a separate worker thread so that a spinning guest
//! does not starve the clock.

use std::time::{Duration, Instant};

use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::{
    task::JoinHandle,
    time::{self, MissedTickBehavior},
};
use wasmtime::{
    Engine, Error as WasmtimeError, Store, Trap, UpdateDeadline,
    component::{Component, HasSelf, Linker, Val},
};

const TICK: Duration = Duration::from_millis(100);
const WAIT: Duration = Duration::from_millis(2000);
const BUDGET_TICKS: u64 = 5;
const BUDGET: Duration = Duration::from_millis(BUDGET_TICKS * TICK.as_millis() as u64);
const MARGIN: Duration = Duration::from_millis(1000);

wasmtime::component::bindgen!({
    inline: r#"
        package test:waiter;

        interface host-waiter {
            wait: func(ms: u32);
            wait-credited: func(ms: u32);
        }

        world test-guest {
            import host-waiter;
        }
    "#,
    imports: { default: async },
});

use test::waiter::host_waiter;

/// The guest WAT component imports the host waiter interface and exports test
/// functions.
///
/// The short loop in `wait_then_work` and `credited_wait_then_work` is
/// essential: Wasmtime checks the epoch deadline only in guest code (at
/// function entry and loop back-edges). A guest returning immediately after a
/// host call without executing any guest code would never be interrupted.
const GUEST_WAT: &str = r#"
(component
  (import "test:waiter/host-waiter" (instance $waiter
    (export "wait" (func (param "ms" u32)))
    (export "wait-credited" (func (param "ms" u32)))
  ))
  (core func $core_wait (canon lower (func $waiter "wait")))
  (core func $core_wait_credited (canon lower (func $waiter "wait-credited")))
  (core module $m
    (import "host" "wait" (func $wait (param i32)))
    (import "host" "wait-credited" (func $wait_credited (param i32)))
    (func $short_loop
      (local $i i32)
      (local.set $i (i32.const 1000))
      (loop $l
        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
        (br_if $l (i32.ne (local.get $i) (i32.const 0)))
      )
    )
    (func $spin
      (loop $l
        (br $l)
      )
    )
    (func (export "wait_then_work") (param $ms i32)
      (call $wait (local.get $ms))
      (call $short_loop)
    )
    (func (export "credited_wait_then_work") (param $ms i32)
      (call $wait_credited (local.get $ms))
      (call $short_loop)
    )
    (func (export "spin")
      (call $spin)
    )
    (func (export "credited_wait_then_spin") (param $ms i32)
      (call $wait_credited (local.get $ms))
      (call $spin)
    )
  )
  (core instance $i (instantiate $m
    (with "host" (instance
      (export "wait" (func $core_wait))
      (export "wait-credited" (func $core_wait_credited))
    ))
  ))
  (func (export "wait-then-work") (param "ms" u32)
    (canon lift (core func $i "wait_then_work"))
  )
  (func (export "credited-wait-then-work") (param "ms" u32)
    (canon lift (core func $i "credited_wait_then_work"))
  )
  (func (export "spin")
    (canon lift (core func $i "spin"))
  )
  (func (export "credited-wait-then-spin") (param "ms" u32)
    (canon lift (core func $i "credited_wait_then_spin"))
  )
)
"#;

struct TestState {
    started: Instant,
    budget: Duration,
    credited: Duration,
    callback_calls: usize,
}

impl TestState {
    fn new(budget: Duration) -> Self {
        Self { started: Instant::now(), budget, credited: Duration::ZERO, callback_calls: 0 }
    }
}

impl host_waiter::Host for TestState {
    async fn wait(&mut self, ms: u32) {
        time::sleep(Duration::from_millis(u64::from(ms))).await;
    }

    async fn wait_credited(&mut self, ms: u32) {
        let sleep_start = Instant::now();
        time::sleep(Duration::from_millis(u64::from(ms))).await;
        self.credited += sleep_start.elapsed();
    }
}

fn on_deadline_at(state: &TestState, now: Instant, tick_duration: Duration) -> UpdateDeadline {
    let elapsed = now.saturating_duration_since(state.started);
    let own_time = elapsed.saturating_sub(state.credited);
    if own_time >= state.budget {
        UpdateDeadline::Interrupt
    } else {
        let remaining = state.budget - own_time;
        let rem_nanos = remaining.as_nanos();
        let tick_nanos = tick_duration.as_nanos();
        let ticks = (rem_nanos.div_ceil(tick_nanos) as u64).max(1);
        UpdateDeadline::Continue(ticks)
    }
}

fn on_deadline(state: &mut TestState) -> UpdateDeadline {
    state.callback_calls += 1;
    on_deadline_at(state, Instant::now(), TICK)
}

fn build_test_engine() -> Engine {
    AppSandboxEngine::build_wasm_engine(Some(10), Some(128 * 1024 * 1024), 4, 4, 4)
        .expect("engine build")
}

fn spawn_ticker(engine: Engine) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    })
}

enum DeadlineSetup {
    TrapAtDeadline,
    CreditCallback,
}

async fn run_guest(
    export_name: &str,
    wait_ms: Option<u32>,
    setup: DeadlineSetup,
) -> (Result<(), WasmtimeError>, Duration, usize) {
    let engine = build_test_engine();
    let ticker = spawn_ticker(engine.clone());

    let mut linker = Linker::new(&engine);
    host_waiter::add_to_linker::<_, HasSelf<TestState>>(&mut linker, |s| s)
        .expect("add host waiter to linker");

    let component = Component::new(&engine, GUEST_WAT).expect("compile wat component");
    let mut store = Store::new(&engine, TestState::new(BUDGET));
    store.set_fuel(100_000_000_000).expect("set fuel");

    match setup {
        DeadlineSetup::TrapAtDeadline => {
            store.epoch_deadline_trap();
        }
        DeadlineSetup::CreditCallback => {
            store.epoch_deadline_callback(|mut ctx| Ok(on_deadline(ctx.data_mut())));
        }
    }

    let instance =
        linker.instantiate_async(&mut store, &component).await.expect("instantiate component");
    let func = instance.get_func(&mut store, export_name).expect("export function not found");

    let params = match wait_ms {
        Some(ms) => vec![Val::U32(ms)],
        None => vec![],
    };
    let mut results = [];

    // Set the epoch deadline and the wall-clock start time together immediately
    // before entering guest code so both start from the same baseline.
    store.set_epoch_deadline(BUDGET_TICKS);
    store.data_mut().started = Instant::now();
    let call_start = Instant::now();
    let res = func.call_async(&mut store, &params, &mut results).await;
    let elapsed = call_start.elapsed();
    let callback_calls = store.data().callback_calls;

    ticker.abort();
    (res, elapsed, callback_calls)
}

#[test]
fn callback_arithmetic_calculates_deadline_credit() {
    let started = Instant::now();
    let state = TestState { started, budget: BUDGET, credited: Duration::ZERO, callback_calls: 0 };

    assert!(matches!(on_deadline_at(&state, started, TICK), UpdateDeadline::Continue(5)));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(250), TICK),
        UpdateDeadline::Continue(3)
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(499), TICK),
        UpdateDeadline::Continue(1)
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(500), TICK),
        UpdateDeadline::Interrupt
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(600), TICK),
        UpdateDeadline::Interrupt
    ));

    let credited_state = TestState {
        started,
        budget: BUDGET,
        credited: Duration::from_millis(1000),
        callback_calls: 0,
    };
    assert!(matches!(
        on_deadline_at(&credited_state, started + Duration::from_millis(1200), TICK),
        UpdateDeadline::Continue(3)
    ));
    assert!(matches!(
        on_deadline_at(&credited_state, started + Duration::from_millis(1500), TICK),
        UpdateDeadline::Interrupt
    ));

    // When credit is greater than elapsed time, own time saturates to zero,
    // granting the full budget.
    let overcredited_state = TestState {
        started,
        budget: BUDGET,
        credited: Duration::from_millis(800),
        callback_calls: 0,
    };
    assert!(matches!(
        on_deadline_at(&overcredited_state, started + Duration::from_millis(500), TICK),
        UpdateDeadline::Continue(5)
    ));
}

/// Proves control behaviour: today an uncredited host call exceeding the epoch
/// budget traps with `Trap::Interrupt` on return to guest code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_host_wait_stops_the_guest_today() {
    let (res, _elapsed, _calls) =
        run_guest("wait-then-work", Some(WAIT.as_millis() as u32), DeadlineSetup::TrapAtDeadline)
            .await;
    let err = res.expect_err("guest should be stopped by deadline trap");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
}

/// Proves that crediting the wait time prevents the guest from trapping on
/// return, and confirms that the extension callback was indeed invoked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_credited_wait_does_not_stop_the_guest() {
    let (res, elapsed, callback_calls) = run_guest(
        "credited-wait-then-work",
        Some(WAIT.as_millis() as u32),
        DeadlineSetup::CreditCallback,
    )
    .await;
    res.expect("credited wait should not stop the guest");
    assert!(callback_calls >= 1, "callback should have run at least once, got {callback_calls}");
    assert!(elapsed >= WAIT, "elapsed {elapsed:?} must be at least wait duration {WAIT:?}");
}

/// Proves that a guest spinning in its own loop is still stopped at its budget
/// even when the extension callback is installed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_loop_is_still_stopped_at_its_budget() {
    let (res, elapsed, _calls) = run_guest("spin", None, DeadlineSetup::CreditCallback).await;
    let err = res.expect_err("spinning guest should be stopped");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
    let min_time = BUDGET - TICK;
    let max_time = BUDGET + MARGIN;
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

/// Proves that wait credit only buys back the time waited, not extra guest
/// time. A guest spinning after a credited wait is interrupted when its budget
/// expires.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credit_buys_back_the_wait_and_no_more() {
    let (res, elapsed, _calls) = run_guest(
        "credited-wait-then-spin",
        Some(WAIT.as_millis() as u32),
        DeadlineSetup::CreditCallback,
    )
    .await;
    let err = res.expect_err("guest should be stopped after budget expires");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
    let min_time = WAIT + BUDGET - TICK;
    let max_time = WAIT + BUDGET + MARGIN;
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

/// Proves that calling an uncredited host function with the callback installed
/// still traps when the wait exceeds the budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wait_without_credit_still_counts() {
    let (res, _elapsed, _calls) =
        run_guest("wait-then-work", Some(WAIT.as_millis() as u32), DeadlineSetup::CreditCallback)
            .await;
    let err = res.expect_err("uncredited wait should stop the guest");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
}
