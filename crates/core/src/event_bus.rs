use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

use dashmap::DashMap;
use rand::RngCore;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use crate::local_registry::EndpointRegistry;

pub const EVENT_FEED_PATH: &str = "/_syneroym/events";
pub const MAX_WINDOW_EVENTS: usize = 10_000;
pub const WINDOW_AGE_MS: i64 = 24 * 60 * 60 * 1000;
pub const READ_LIMIT_CAP: usize = 500;
pub const MAX_FEEDS_PER_APP_INSTANCE: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub run_id: String,
    pub seq: u64,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.run_id, self.seq)
    }
}

impl Cursor {
    pub fn parse(s: &str) -> Result<Self, EventError> {
        let Some((run_id, seq_str)) = s.rsplit_once('-') else {
            return Err(EventError::InvalidArgument("cursor".to_string()));
        };
        if run_id.len() != 16
            || !run_id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        {
            return Err(EventError::InvalidArgument("cursor".to_string()));
        }
        let Ok(seq) = seq_str.parse::<u64>() else {
            return Err(EventError::InvalidArgument("cursor".to_string()));
        };
        Ok(Self { run_id: run_id.to_string(), seq })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub cursor: Cursor,
    pub kind: String,
    pub service: String,
    pub subject: String,
    pub item: Option<String>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Events { events: Vec<Event>, last: Cursor },
    Reload { now: Cursor },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventError {
    #[error("the writer is not part of an app instance")]
    NoAppInstance,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

struct WindowInner {
    events: VecDeque<Event>,
    next_seq: u64,
}

struct Window {
    inner: Mutex<WindowInner>,
    changed_tx: watch::Sender<u64>,
    feed_permits: Arc<Semaphore>,
}

impl Window {
    fn new() -> Self {
        let (changed_tx, _) = watch::channel(0);
        Self {
            inner: Mutex::new(WindowInner { events: VecDeque::new(), next_seq: 1 }),
            changed_tx,
            feed_permits: Arc::new(Semaphore::new(MAX_FEEDS_PER_APP_INSTANCE)),
        }
    }
}

pub struct EventBus {
    registry: EndpointRegistry,
    run_id: String,
    windows: DashMap<String, Arc<Window>>,
}

impl fmt::Debug for EventBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventBus")
            .field("run_id", &self.run_id)
            .field("windows_count", &self.windows.len())
            .finish()
    }
}

impl EventBus {
    pub fn new(registry: EndpointRegistry) -> Arc<Self> {
        let mut rng = rand::rng();
        let mut bytes = [0u8; 8];
        rng.fill_bytes(&mut bytes);
        let run_id = hex::encode(bytes);

        Arc::new(Self { registry, run_id, windows: DashMap::new() })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn app_instance_of(&self, service_id: &str) -> Option<String> {
        self.registry.app_context_of(service_id).map(|(app_instance, _)| app_instance)
    }

    pub fn feed_semaphore(&self, app_instance: &str) -> Arc<Semaphore> {
        let window = self.get_or_create_window(app_instance);
        window.feed_permits.clone()
    }

    pub fn try_acquire_feed_permit(&self, app_instance: &str) -> Option<OwnedSemaphorePermit> {
        self.feed_semaphore(app_instance).try_acquire_owned().ok()
    }

    pub fn head(&self, app_instance: &str) -> Cursor {
        if let Some(window) = self.windows.get(app_instance) {
            let inner = window.inner.lock().unwrap_or_else(PoisonError::into_inner);
            Cursor { run_id: self.run_id.clone(), seq: inner.next_seq.saturating_sub(1) }
        } else {
            Cursor { run_id: self.run_id.clone(), seq: 0 }
        }
    }

    pub fn changed(&self, app_instance: &str) -> watch::Receiver<u64> {
        let window = self.get_or_create_window(app_instance);
        window.changed_tx.subscribe()
    }

    pub fn record(
        &self,
        service_id: &str,
        kind: &str,
        subject: &str,
        item: Option<&str>,
        now_ms: i64,
    ) -> Result<Cursor, EventError> {
        validate_record_args(kind, subject, item)?;

        let Some((app_instance, service_name)) = self.registry.app_context_of(service_id) else {
            return Err(EventError::NoAppInstance);
        };

        let window = self.get_or_create_window(&app_instance);
        let mut inner = window.inner.lock().unwrap_or_else(PoisonError::into_inner);
        evict_window(&mut inner.events, now_ms);

        let seq = inner.next_seq;
        inner.next_seq = inner.next_seq.saturating_add(1);

        let cursor = Cursor { run_id: self.run_id.clone(), seq };

        let event = Event {
            cursor: cursor.clone(),
            kind: kind.to_string(),
            service: service_name,
            subject: subject.to_string(),
            item: item.map(ToString::to_string),
            at_ms: now_ms,
        };

        inner.events.push_back(event);
        let _ = window.changed_tx.send(seq);

        Ok(cursor)
    }

    pub fn read(
        &self,
        app_instance: &str,
        after: Option<&Cursor>,
        limit: usize,
        now_ms: i64,
    ) -> ReadOutcome {
        let window = self.get_or_create_window(app_instance);
        let mut inner = window.inner.lock().unwrap_or_else(PoisonError::into_inner);
        evict_window(&mut inner.events, now_ms);

        let head_seq = inner.next_seq.saturating_sub(1);
        let head = Cursor { run_id: self.run_id.clone(), seq: head_seq };

        let oldest = inner.events.front().map(|e| e.cursor.seq).unwrap_or(inner.next_seq);

        let Some(after_cursor) = after else {
            return ReadOutcome::Events { events: Vec::new(), last: head };
        };

        if after_cursor.run_id != self.run_id
            || after_cursor.seq > head_seq
            || after_cursor.seq.saturating_add(1) < oldest
        {
            return ReadOutcome::Reload { now: head };
        }

        if limit == 0 {
            return ReadOutcome::Events { events: Vec::new(), last: after_cursor.clone() };
        }

        let effective_limit = limit.min(READ_LIMIT_CAP);
        let matched_events: Vec<Event> = inner
            .events
            .iter()
            .filter(|e| e.cursor.seq > after_cursor.seq)
            .take(effective_limit)
            .cloned()
            .collect();

        let last =
            matched_events.last().map(|e| e.cursor.clone()).unwrap_or_else(|| after_cursor.clone());

        ReadOutcome::Events { events: matched_events, last }
    }

    fn get_or_create_window(&self, app_instance: &str) -> Arc<Window> {
        self.windows
            .entry(app_instance.to_string())
            .or_insert_with(|| Arc::new(Window::new()))
            .clone()
    }
}

fn validate_record_args(kind: &str, subject: &str, item: Option<&str>) -> Result<(), EventError> {
    if kind.is_empty() || kind.len() > 64 {
        return Err(EventError::InvalidArgument("kind".to_string()));
    }
    let first = kind.chars().next().unwrap_or('\0');
    if !first.is_ascii_lowercase() {
        return Err(EventError::InvalidArgument("kind".to_string()));
    }
    if !kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-') {
        return Err(EventError::InvalidArgument("kind".to_string()));
    }

    if subject.is_empty() || subject.len() > 128 || subject.chars().any(|c| c.is_ascii_control()) {
        return Err(EventError::InvalidArgument("subject".to_string()));
    }

    if item
        .is_some_and(|s| s.is_empty() || s.len() > 128 || s.chars().any(|c| c.is_ascii_control()))
    {
        return Err(EventError::InvalidArgument("item".to_string()));
    }

    Ok(())
}

fn evict_window(events: &mut VecDeque<Event>, now_ms: i64) {
    let cutoff = now_ms.saturating_sub(WINDOW_AGE_MS);
    while events.len() > MAX_WINDOW_EVENTS || events.front().is_some_and(|e| e.at_ms < cutoff) {
        events.pop_front();
    }
}

#[cfg(test)]
mod tests;
