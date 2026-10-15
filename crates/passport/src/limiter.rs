//! A per-address request limit for the registration endpoints.

use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv6Addr},
    sync::{Mutex, PoisonError},
};

/// Requests one source may make in one window.
pub const REQUESTS_PER_MINUTE: u32 = 10;
const WINDOW_SECS: u64 = 60;
const MAX_TRACKED_SOURCES: usize = 10_000;

/// A fixed-window request counter per source, in memory. A source is an IPv4
/// address or an IPv6 `/64`: one sender owns a whole `/64`, so counting the
/// full address would give it unlimited limits. When the table is full, the
/// source whose window started earliest is dropped to make room. A sender
/// that wants to reset its own counter this way must first send from
/// `max_sources` newer sources.
///
/// The window is fixed, so a source can make up to twice the limit in a short
/// time across the edge of two windows.
#[derive(Debug)]
pub struct RequestLimiter {
    per_window: u32,
    max_sources: usize,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    windows: HashMap<IpAddr, Window>,
    /// Window starts in time order. An entry whose window was replaced or
    /// dropped is stale and is skipped when it reaches the front.
    order: VecDeque<(IpAddr, u64)>,
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

/// The address a limit is counted for: IPv4-mapped IPv6 as IPv4, other IPv6
/// as its `/64`.
fn source_key(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & (!0u128 << 64))),
        v4 => v4,
    }
}

impl State {
    /// Forget the window at the front of `order`. Returns false when `order`
    /// is empty.
    fn pop_oldest(&mut self) -> bool {
        let Some((source, started_at)) = self.order.pop_front() else {
            return false;
        };
        if self.windows.get(&source).is_some_and(|w| w.started_at == started_at) {
            self.windows.remove(&source);
        }
        true
    }

    /// Drop the windows that have ended: they are at the front.
    fn drop_ended(&mut self, now_secs: u64) {
        while let Some(&(_, started_at)) = self.order.front() {
            if now_secs < started_at.saturating_add(WINDOW_SECS) {
                break;
            }
            self.pop_oldest();
        }
    }
}

impl RequestLimiter {
    fn new(per_window: u32, max_sources: usize) -> Self {
        Self { per_window, max_sources, state: Mutex::default() }
    }

    /// Count one request from `source`. False when the source has used its
    /// requests for this window.
    pub fn allow(&self, source: IpAddr, now_secs: u64) -> bool {
        let source = source_key(source);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(window) = state.windows.get_mut(&source)
            && now_secs < window.started_at.saturating_add(WINDOW_SECS)
        {
            if window.count >= self.per_window {
                return false;
            }
            window.count += 1;
            return true;
        }
        state.drop_ended(now_secs);
        while state.windows.len() >= self.max_sources && state.pop_oldest() {}
        state.windows.insert(source, Window { started_at: now_secs, count: 1 });
        state.order.push_back((source, now_secs));
        true
    }
}

#[cfg(test)]
mod tests;
