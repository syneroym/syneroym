use std::time::Duration;
use sysinfo::System;
use tokio::time;
use tracing::error;

#[derive(Debug)]
pub struct SystemSampler {
    interval: Duration,
}

impl SystemSampler {
    pub fn new(interval: Duration) -> Self {
        Self { interval }
    }

    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut sys = System::new();
            let pid = match sysinfo::get_current_pid() {
                Ok(pid) => pid,
                Err(e) => {
                    error!("Failed to get current process PID: {:?}", e);
                    return;
                }
            };

            let mut interval = time::interval(self.interval);
            loop {
                interval.tick().await;

                // Refresh process info
                sys.refresh_process(pid);
                if let Some(process) = sys.process(pid) {
                    // sysinfo v0.30 returns memory in bytes
                    let rss = process.memory();
                    let cpu = process.cpu_usage();

                    metrics::gauge!("substrate.system.rss_bytes").set(rss as f64);
                    metrics::gauge!("substrate.system.cpu_percent").set(cpu as f64);
                }

                // Count open FDs
                let fds = std::fs::read_dir("/dev/fd").map(|dir| dir.count()).unwrap_or(0);
                metrics::gauge!("substrate.system.open_fds").set(fds as f64);

                // Count active tokio tasks
                let tokio_metrics = tokio::runtime::Handle::current().metrics();
                let active_tasks = tokio_metrics.num_alive_tasks();
                metrics::gauge!("substrate.tokio.active_tasks").set(active_tasks as f64);
            }
        })
    }
}
