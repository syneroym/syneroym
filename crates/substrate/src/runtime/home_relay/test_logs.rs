//! Shared test logging capture helper.

use std::{
    io,
    sync::{Arc, Mutex},
};

use tracing::subscriber::{self, DefaultGuard};
use tracing_subscriber::{fmt, prelude::*};

pub(super) struct MockWriter {
    pub(super) logs: Arc<Mutex<Vec<u8>>>,
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
pub(super) fn capture() -> (Arc<Mutex<Vec<u8>>>, DefaultGuard) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = logs.clone();
    let layer =
        fmt::layer().with_ansi(false).with_writer(move || MockWriter { logs: writer.clone() });
    (logs, subscriber::set_default(tracing_subscriber::registry().with(layer)))
}

pub(super) fn lines_with(logs: &Mutex<Vec<u8>>, level: &str) -> usize {
    let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    text.lines().filter(|l| l.contains(level)).count()
}

pub(super) fn warnings_about(logs: &Mutex<Vec<u8>>, text: &str) -> usize {
    let text_of_logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    text_of_logs.lines().filter(|l| l.contains("WARN") && l.contains(text)).count()
}
