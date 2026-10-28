#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Pins the Wasmtime epoch-callback behaviour that the batched outbound call
//! relies on.
//!
//! These tests verify that host calls can be credited against the guest epoch
//! deadline, so a guest waiting on slow outbound calls is not interrupted,
//! while a guest looping in its own code is still interrupted on time.

use std::time::{Duration, Instant};

use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::time::{self, MissedTickBehavior};
use wasmtime::{
    Engine, Store, Trap, UpdateDeadline,
    component::{Component, HasSelf, Linker, Val},
};

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
}

impl TestState {
    fn new(budget: Duration) -> Self {
        Self { started: Instant::now(), budget, credited: Duration::ZERO }
    }
}

impl test::waiter::host_waiter::Host for TestState {
    async fn wait(&mut self, ms: u32) {
        time::sleep(Duration::from_millis(u64::from(ms))).await;
    }

    async fn wait_credited(&mut self, ms: u32) {
        let duration = Duration::from_millis(u64::from(ms));
        time::sleep(duration).await;
        self.credited += duration;
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

fn on_deadline(state: &TestState) -> UpdateDeadline {
    on_deadline_at(state, Instant::now(), Duration::from_millis(100))
}

fn build_test_engine() -> Engine {
    AppSandboxEngine::build_wasm_engine(Some(10), Some(128 * 1024 * 1024), 4, 4, 4)
        .expect("engine build")
}

fn spawn_ticker(engine: Engine) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(100));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    })
}

enum DeadlineSetup {
    Trap,
    Callback,
}

async fn run_guest(
    export_name: &str,
    wait_ms: Option<u32>,
    budget_ticks: u64,
    setup: DeadlineSetup,
) -> (Result<(), wasmtime::Error>, Duration) {
    let engine = build_test_engine();
    let ticker = spawn_ticker(engine.clone());

    let mut linker = Linker::new(&engine);
    test::waiter::host_waiter::add_to_linker::<_, HasSelf<TestState>>(&mut linker, |s| s)
        .expect("add host waiter to linker");

    let component = Component::new(&engine, GUEST_WAT).expect("compile wat component");
    let budget = Duration::from_millis(budget_ticks * 100);
    let mut store = Store::new(&engine, TestState::new(budget));
    store.set_fuel(100_000_000_000).expect("set fuel");

    match setup {
        DeadlineSetup::Trap => {
            store.epoch_deadline_trap();
            store.set_epoch_deadline(budget_ticks);
        }
        DeadlineSetup::Callback => {
            store.epoch_deadline_callback(|ctx| Ok(on_deadline(ctx.data())));
            store.set_epoch_deadline(budget_ticks);
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

    store.data_mut().started = Instant::now();
    let call_start = Instant::now();
    let res = func.call_async(&mut store, &params, &mut results).await;
    let elapsed = call_start.elapsed();

    ticker.abort();
    (res, elapsed)
}

#[test]
fn callback_arithmetic_calculates_deadline_credit() {
    let started = Instant::now();
    let tick = Duration::from_millis(100);
    let budget = Duration::from_millis(500);

    let state = TestState { started, budget, credited: Duration::ZERO };

    assert!(matches!(on_deadline_at(&state, started, tick), UpdateDeadline::Continue(5)));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(250), tick),
        UpdateDeadline::Continue(3)
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(499), tick),
        UpdateDeadline::Continue(1)
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(500), tick),
        UpdateDeadline::Interrupt
    ));
    assert!(matches!(
        on_deadline_at(&state, started + Duration::from_millis(600), tick),
        UpdateDeadline::Interrupt
    ));

    let credited_state = TestState { started, budget, credited: Duration::from_millis(1000) };
    assert!(matches!(
        on_deadline_at(&credited_state, started + Duration::from_millis(1200), tick),
        UpdateDeadline::Continue(3)
    ));
    assert!(matches!(
        on_deadline_at(&credited_state, started + Duration::from_millis(1500), tick),
        UpdateDeadline::Interrupt
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_host_wait_stops_the_guest_today() {
    let (res, _elapsed) = run_guest("wait-then-work", Some(2000), 5, DeadlineSetup::Trap).await;
    let err = res.expect_err("guest should be stopped by deadline trap");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_credited_wait_does_not_stop_the_guest() {
    let (res, _elapsed) =
        run_guest("credited-wait-then-work", Some(2000), 5, DeadlineSetup::Callback).await;
    res.expect("credited wait should not stop the guest");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_loop_is_still_stopped_at_its_budget() {
    let (res, elapsed) = run_guest("spin", None, 5, DeadlineSetup::Callback).await;
    let err = res.expect_err("spinning guest should be stopped");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
    let min_time = Duration::from_millis(400);
    let max_time = Duration::from_millis(1250);
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credit_buys_back_the_wait_and_no_more() {
    let (res, elapsed) =
        run_guest("credited-wait-then-spin", Some(2000), 5, DeadlineSetup::Callback).await;
    let err = res.expect_err("guest should be stopped after budget expires");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
    let min_time = Duration::from_millis(2400);
    let max_time = Duration::from_millis(3250);
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wait_without_credit_still_counts() {
    let (res, _elapsed) = run_guest("wait-then-work", Some(2000), 5, DeadlineSetup::Callback).await;
    let err = res.expect_err("uncredited wait should stop the guest");
    assert!(
        matches!(err.downcast_ref::<Trap>(), Some(Trap::Interrupt)),
        "expected Trap::Interrupt, got {err:?}"
    );
}
