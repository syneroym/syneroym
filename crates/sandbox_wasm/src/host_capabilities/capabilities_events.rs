use std::time::{SystemTime, UNIX_EPOCH};

use syneroym_core::event_bus::{Cursor, EventError, ReadOutcome};
use syneroym_wit_interfaces::events_host::syneroym::events::events::{
    Event as WitEvent, EventsError, Host as EventsHost, ReadResult,
};

use crate::host_capabilities::HostState;

impl HostState {
    pub fn record_event(
        &self,
        kind: String,
        subject: String,
        item: Option<String>,
    ) -> Result<String, EventsError> {
        if self.read_only {
            metrics::counter!("substrate.events.refused", "reason" => "read-only").increment(1);
            return Err(EventsError::ReadOnly);
        }
        if self.app_instance_id.is_none() {
            metrics::counter!("substrate.events.refused", "reason" => "no-app-instance")
                .increment(1);
            return Err(EventsError::NoAppInstance);
        }
        let Some(bus) = self.event_bus.as_ref() else {
            metrics::counter!("substrate.events.refused", "reason" => "no-app-instance")
                .increment(1);
            return Err(EventsError::NoAppInstance);
        };

        let now_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);

        let cursor = bus
            .record(&self.component_id, &kind, &subject, item.as_deref(), now_ms)
            .map_err(|e| match e {
                EventError::NoAppInstance => {
                    metrics::counter!("substrate.events.refused", "reason" => "no-app-instance")
                        .increment(1);
                    EventsError::NoAppInstance
                }
                EventError::InvalidArgument(msg) => {
                    metrics::counter!("substrate.events.refused", "reason" => "invalid-argument")
                        .increment(1);
                    EventsError::InvalidArgument(msg)
                }
            })?;

        metrics::counter!("substrate.events.recorded").increment(1);
        Ok(cursor.to_string())
    }

    pub fn read_events(
        &self,
        after: Option<String>,
        limit: u32,
    ) -> Result<ReadResult, EventsError> {
        let Some(app_instance) = self.app_instance_id.as_deref() else {
            return Err(EventsError::NoAppInstance);
        };
        let Some(bus) = self.event_bus.as_ref() else {
            return Err(EventsError::NoAppInstance);
        };

        let cursor = if let Some(after_str) = after.as_deref() {
            let parsed = Cursor::parse(after_str).map_err(|e| match e {
                EventError::InvalidArgument(msg) => EventsError::InvalidArgument(msg),
                EventError::NoAppInstance => EventsError::NoAppInstance,
            })?;
            Some(parsed)
        } else {
            None
        };

        let now_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);

        let outcome = bus.read(app_instance, cursor.as_ref(), limit as usize, now_ms);
        match outcome {
            ReadOutcome::Events { events, last } => {
                let wit_events = events
                    .into_iter()
                    .map(|e| WitEvent {
                        cursor: e.cursor.to_string(),
                        kind: e.kind,
                        service: e.service,
                        subject: e.subject,
                        item: e.item,
                        at_ms: e.at_ms,
                    })
                    .collect();
                Ok(ReadResult::Events((wit_events, last.to_string())))
            }
            ReadOutcome::Reload { now } => {
                metrics::counter!("substrate.events.reloads").increment(1);
                Ok(ReadResult::Reload(now.to_string()))
            }
        }
    }
}

impl EventsHost for HostState {
    async fn record(
        &mut self,
        kind: String,
        subject: String,
        item: Option<String>,
    ) -> Result<String, EventsError> {
        self.record_event(kind, subject, item)
    }

    async fn read(&mut self, after: Option<String>, limit: u32) -> Result<ReadResult, EventsError> {
        self.read_events(after, limit)
    }
}
