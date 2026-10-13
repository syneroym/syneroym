//! How often the resolver warns.

use std::{
    io,
    sync::{Arc, Mutex},
};

use tokio::time::Instant;
use tracing::subscriber::{self, DefaultGuard};
use tracing_subscriber::{fmt, prelude::*};

use super::{FakeIo, at, entry, secs, start};

struct MockWriter {
    logs: Arc<Mutex<Vec<u8>>>,
}

impl io::Write for MockWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.logs.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Logs of the current thread, until the guard drops.
fn capture() -> (Arc<Mutex<Vec<u8>>>, DefaultGuard) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = logs.clone();
    let layer =
        fmt::layer().with_ansi(false).with_writer(move || MockWriter { logs: writer.clone() });
    (logs, subscriber::set_default(tracing_subscriber::registry().with(layer)))
}

fn lines_with(logs: &Mutex<Vec<u8>>, level: &str) -> usize {
    let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    text.lines().filter(|l| l.contains(level)).count()
}

#[tokio::test(start_paused = true)]
async fn degraded_mode_warns_once_per_five_minutes() {
    let (logs, _guard) = capture();
    let io = FakeIo::new();
    let t0 = Instant::now();
    let run = start(&io, None);

    // About seven tries by now, and one warning.
    at(t0, 280).await;
    assert!(io.fetches() >= 6, "{} tries", io.fetches());
    assert_eq!(lines_with(&logs, "WARN"), 1);

    // Retries wait 5, 10, ... 600 s, each with 10 % jitter. So the second
    // warning comes at 300 s or later, and a third only after 600 s.
    at(t0, 700).await;
    assert!((2..=3).contains(&lines_with(&logs, "WARN")), "{}", lines_with(&logs, "WARN"));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn relay_set_outside_the_resolver_is_not_called_degraded() {
    let (logs, _guard) = capture();
    let io = FakeIo::new();
    io.set_current(&entry(1));
    let run = start(&io, None);

    secs(100).await;

    assert!(io.fetches() > 1, "the resolver keeps trying");
    assert_eq!(io.clears(), 0);
    assert_eq!(lines_with(&logs, "no home relay"), 0);
    assert!(lines_with(&logs, "set outside") >= 1);
    run.stop().await;
}
