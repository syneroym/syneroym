use std::sync::{Arc, OnceLock, RwLock};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Default)]
struct SharedLogBuffer {
    lines: Arc<RwLock<Vec<String>>>,
}

struct LogWriter {
    buffer: Arc<RwLock<Vec<String>>>,
}

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let mut lines = self.buffer.write().unwrap();
            lines.push(s.to_string());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
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
        let filter = tracing_subscriber::filter::EnvFilter::new("trace");
        let _ = tracing_subscriber::registry().with(layer).with(filter).try_init();
        SharedLogBuffer { lines }
    });
    shared.lines.clone()
}
