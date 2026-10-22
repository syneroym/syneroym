use std::{
    io::{self, Write},
    str,
    sync::{Arc, OnceLock, RwLock},
};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Default)]
struct SharedLogBuffer {
    lines: Arc<RwLock<Vec<String>>>,
}

struct LogWriter {
    buffer: Arc<RwLock<Vec<String>>>,
}

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(s) = str::from_utf8(buf)
            && let Ok(mut lines) = self.buffer.write()
        {
            lines.push(s.to_string());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

static SHARED_LOGS: OnceLock<SharedLogBuffer> = OnceLock::new();

pub fn init_trace_collector() -> Arc<RwLock<Vec<String>>> {
    let shared = SHARED_LOGS.get_or_init(|| {
        let lines = Arc::new(RwLock::new(Vec::new()));
        let lines_clone = lines.clone();
        let make_writer = move || LogWriter { buffer: lines_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
        // Every syneroym crate in full, for the leak checks; other crates
        // (iroh, quinn) only at `warn`, so a multi-node test does not hold
        // their trace output in memory.
        let filter = tracing_subscriber::filter::EnvFilter::new("warn,syneroym=trace");
        let _ = tracing_subscriber::registry().with(layer).with(filter).try_init();
        SharedLogBuffer { lines }
    });
    shared.lines.clone()
}

/// The `Forwarding preamble` lines a coordinator logged for `did`. Asserts
/// there is at least one, that each asks for the end-to-end channel, and
/// that none carries the caller's identity.
pub fn forwarded_without_identity(logs: &RwLock<Vec<String>>, did: &str) -> Vec<String> {
    let captured = logs.read().unwrap();
    let lines: Vec<String> = captured
        .iter()
        .filter(|l| l.contains("Forwarding preamble") && l.contains(did))
        .cloned()
        .collect();
    assert!(!lines.is_empty(), "no Forwarding preamble line for {did}");
    for line in &lines {
        assert!(line.contains("enc=ecdh-p256"), "must have enc: {line}");
        assert!(line.contains("master=-"), "must have master=-: {line}");
        assert!(line.contains("sig=-"), "must have sig=-: {line}");
        assert!(!line.contains("pubkey="), "must not have pubkey=: {line}");
    }
    lines
}
