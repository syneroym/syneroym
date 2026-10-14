//! A per-address request limit for the registration endpoints.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, PoisonError},
};

/// Requests one source address may make in one minute.
pub const REQUESTS_PER_MINUTE: u32 = 10;
const WINDOW_SECS: u64 = 60;
const MAX_TRACKED_SOURCES: usize = 10_000;

/// A fixed-window request counter per source address, in memory. It keys on
/// the full address. When it tracks too many addresses it refuses a new one
/// (fails closed) instead of growing.
#[derive(Debug)]
pub struct RequestLimiter {
    per_window: u32,
    max_sources: usize,
    windows: Mutex<HashMap<IpAddr, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started_at: u64,
    count: u32,
}

impl Default for RequestLimiter {
    fn default() -> Self {
        Self::new(REQUESTS_PER_MINUTE, MAX_TRACKED_SOURCES)
    }
}

impl RequestLimiter {
    fn new(per_window: u32, max_sources: usize) -> Self {
        Self { per_window, max_sources, windows: Mutex::default() }
    }

    /// Count one request from `source`. False when the source has used its
    /// requests for this minute, or when too many sources are tracked.
    pub fn allow(&self, source: IpAddr, now_secs: u64) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let running = |w: &Window| now_secs < w.started_at.saturating_add(WINDOW_SECS);
        if let Some(window) = windows.get_mut(&source)
            && running(window)
        {
            if window.count >= self.per_window {
                return false;
            }
            window.count += 1;
            return true;
        }
        if !windows.contains_key(&source) && windows.len() >= self.max_sources {
            windows.retain(|_, w| running(w));
            if windows.len() >= self.max_sources {
                return false;
            }
        }
        windows.insert(source, Window { started_at: now_secs, count: 1 });
        true
    }
}

#[cfg(test)]
mod tests;
