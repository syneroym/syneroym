use syneroym_app_host::types::events::{
    Event as GuestEvent, EventsError as GuestEventsError, ReadResult as GuestReadResult,
};
use syneroym_wit_interfaces::events_host::syneroym::events::events::{
    EventsError as HostEventsError, ReadResult as HostReadResult,
};

pub(crate) fn events_error_guest(e: HostEventsError) -> GuestEventsError {
    match e {
        HostEventsError::NoAppInstance => GuestEventsError::NoAppInstance,
        HostEventsError::InvalidArgument(msg) => GuestEventsError::InvalidArgument(msg),
        HostEventsError::ReadOnly => GuestEventsError::ReadOnly,
    }
}

pub(crate) fn read_result_guest(r: HostReadResult) -> GuestReadResult {
    match r {
        HostReadResult::Events((events, cursor)) => {
            let guest_events = events
                .into_iter()
                .map(|e| GuestEvent {
                    cursor: e.cursor,
                    kind: e.kind,
                    service: e.service,
                    subject: e.subject,
                    item: e.item,
                    at_ms: e.at_ms,
                })
                .collect();
            GuestReadResult::Events((guest_events, cursor))
        }
        HostReadResult::Reload(cursor) => GuestReadResult::Reload(cursor),
    }
}
