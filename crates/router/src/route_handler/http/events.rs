use std::{
    collections::VecDeque,
    convert::Infallible,
    result,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    HeaderMap, Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HeaderValue, RETRY_AFTER},
};
use serde::Serialize;
use syneroym_core::{
    event_bus::{Cursor, Event, EventBus, READ_LIMIT_CAP, ReadOutcome},
    protocol_utils::SessionRevocationCheck,
};
use tokio::sync::{OwnedSemaphorePermit, watch};

use super::{HttpBody, HttpHandler, SESSION_CALLER_MARKER, http_error};

#[cfg(test)]
mod tests;

pub(crate) struct FeedGuard;

impl FeedGuard {
    pub(crate) fn new() -> Self {
        metrics::gauge!("substrate.events.feeds_open").increment(1.0);
        Self
    }
}

impl Drop for FeedGuard {
    fn drop(&mut self) {
        metrics::gauge!("substrate.events.feeds_open").decrement(1.0);
    }
}

pub const FEED_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);

#[derive(Debug, PartialEq, Eq)]
enum StartCursorState {
    None,
    Cursor(Cursor),
    Reload,
}

fn parse_start_cursor(headers: &HeaderMap, query: Option<&str>) -> StartCursorState {
    let header_val = headers.get("Last-Event-ID").and_then(|v| v.to_str().ok());

    if let Some(val) = header_val {
        return match Cursor::parse(val) {
            Ok(cursor) => StartCursorState::Cursor(cursor),
            Err(_) => StartCursorState::Reload,
        };
    }

    if let Some(q) = query
        && let Some(val) = parse_after_query_param(q)
    {
        return match Cursor::parse(&val) {
            Ok(cursor) => StartCursorState::Cursor(cursor),
            Err(_) => StartCursorState::Reload,
        };
    }

    StartCursorState::None
}

fn parse_after_query_param(query: &str) -> Option<String> {
    for part in query.split('&') {
        if let Some((k, v)) = part.split_once('=')
            && k == "after"
        {
            return percent_encoding::percent_decode_str(v)
                .decode_utf8()
                .ok()
                .map(|s| s.into_owned());
        }
    }
    None
}

#[derive(Serialize)]
struct EventData<'a> {
    cursor: &'a str,
    kind: &'a str,
    service: &'a str,
    subject: &'a str,
    item: Option<&'a str>,
    at_ms: i64,
}

#[derive(Serialize)]
struct CursorData<'a> {
    cursor: &'a str,
}

fn format_ready_frame(head: &Cursor) -> String {
    let head_str = head.to_string();
    let data = CursorData { cursor: &head_str };
    let json = serde_json::to_string(&data).unwrap_or_default();
    format!("event: ready\nid: {head_str}\ndata: {json}\n\n")
}

fn format_reload_frame(now: &Cursor) -> String {
    let now_str = now.to_string();
    let data = CursorData { cursor: &now_str };
    let json = serde_json::to_string(&data).unwrap_or_default();
    format!("event: reload\nid: {now_str}\ndata: {json}\n\n")
}

fn format_change_frame(ev: &Event) -> String {
    let cursor_str = ev.cursor.to_string();
    let data = EventData {
        cursor: &cursor_str,
        kind: &ev.kind,
        service: &ev.service,
        subject: &ev.subject,
        item: ev.item.as_deref(),
        at_ms: ev.at_ms,
    };
    let json = serde_json::to_string(&data).unwrap_or_default();
    format!("event: change\nid: {cursor_str}\ndata: {json}\n\n")
}

fn fetch_events(
    bus: &EventBus,
    app_instance: &str,
    last_cursor: &mut Cursor,
    buffered_frames: &mut VecDeque<Bytes>,
    now_ms: i64,
) {
    loop {
        let outcome = bus.read(app_instance, Some(last_cursor), READ_LIMIT_CAP, now_ms);
        match outcome {
            ReadOutcome::Reload { now } => {
                metrics::counter!("substrate.events.reloads").increment(1);
                let frame = format_reload_frame(&now);
                buffered_frames.push_back(Bytes::from(frame));
                *last_cursor = now;
                break;
            }
            ReadOutcome::Events { events, last } => {
                let count = events.len();
                for ev in &events {
                    let frame = format_change_frame(ev);
                    buffered_frames.push_back(Bytes::from(frame));
                }
                *last_cursor = last;
                if count < READ_LIMIT_CAP {
                    break;
                }
            }
        }
    }
}

fn current_time_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn is_session_expired_or_revoked(
    token: Option<&str>,
    exp_secs: Option<u64>,
    revocation: Option<&Arc<dyn SessionRevocationCheck>>,
) -> bool {
    if let Some(exp) = exp_secs {
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if now_secs >= exp {
            return true;
        }
    }
    if let (Some(t), Some(r)) = (token, revocation)
        && r.is_revoked(t)
    {
        return true;
    }
    false
}

struct FeedState {
    bus: Arc<EventBus>,
    app_instance: String,
    last_cursor: Cursor,
    buffered_frames: VecDeque<Bytes>,
    changed_rx: watch::Receiver<u64>,
    session_token: Option<String>,
    session_exp_secs: Option<u64>,
    session_revocation: Option<Arc<dyn SessionRevocationCheck>>,
    _permit: OwnedSemaphorePermit,
    _guard: FeedGuard,
}

fn make_feed_stream(state: FeedState) -> HttpBody {
    let stream = stream::unfold(state, |mut state| async move {
        while state.buffered_frames.is_empty() {
            tokio::select! {
                res = state.changed_rx.changed() => {
                    if res.is_err() {
                        return None;
                    }
                    let _ = state.changed_rx.borrow_and_update();
                    if is_session_expired_or_revoked(
                        state.session_token.as_deref(),
                        state.session_exp_secs,
                        state.session_revocation.as_ref(),
                    ) {
                        return None;
                    }
                    let now_ms = current_time_ms();
                    fetch_events(
                        &state.bus,
                        &state.app_instance,
                        &mut state.last_cursor,
                        &mut state.buffered_frames,
                        now_ms,
                    );
                }
                _ = tokio::time::sleep(FEED_KEEP_ALIVE_INTERVAL) => {
                    if is_session_expired_or_revoked(
                        state.session_token.as_deref(),
                        state.session_exp_secs,
                        state.session_revocation.as_ref(),
                    ) {
                        return None;
                    }
                    state.buffered_frames.push_back(Bytes::from_static(b": keep-alive\n\n"));
                }
            }
        }
        let frame = state.buffered_frames.pop_front()?;
        Some((Ok::<_, Infallible>(Frame::data(frame)), state))
    });

    StreamBody::new(stream).boxed_unsync()
}

fn init_initial_buffered_frames(
    bus: &EventBus,
    app_instance: &str,
    cursor_state: StartCursorState,
    now_ms: i64,
) -> (Cursor, VecDeque<Bytes>) {
    let mut buffered_frames = VecDeque::new();
    let mut last_cursor = bus.head(app_instance);
    match cursor_state {
        StartCursorState::None => {
            let frame = format_ready_frame(&last_cursor);
            buffered_frames.push_back(Bytes::from(frame));
        }
        StartCursorState::Reload => {
            metrics::counter!("substrate.events.reloads").increment(1);
            let frame = format_reload_frame(&last_cursor);
            buffered_frames.push_back(Bytes::from(frame));
        }
        StartCursorState::Cursor(cursor) => {
            last_cursor = cursor;
            fetch_events(bus, app_instance, &mut last_cursor, &mut buffered_frames, now_ms);
        }
    }
    (last_cursor, buffered_frames)
}

#[derive(Debug, Clone, Copy)]
enum FeedAccessError {
    Unauthorized,
    Forbidden,
    NotAppService,
    NotAcceptable,
}

impl FeedAccessError {
    fn into_response(self) -> Response<HttpBody> {
        match self {
            Self::Unauthorized => http_error(StatusCode::UNAUTHORIZED, "Unauthorized".into()),
            Self::Forbidden => http_error(StatusCode::FORBIDDEN, "Forbidden".into()),
            Self::NotAppService => {
                http_error(StatusCode::NOT_FOUND, "this service is not part of an app".into())
            }
            Self::NotAcceptable => http_error(
                StatusCode::NOT_ACCEPTABLE,
                "Accept: text/event-stream is required".into(),
            ),
        }
    }
}

impl HttpHandler {
    fn check_feed_access(
        &self,
        bus: &EventBus,
        headers: &HeaderMap,
    ) -> result::Result<String, FeedAccessError> {
        let is_session = self
            .caller
            .as_ref()
            .is_some_and(|c| c.session.claims.contains_key(SESSION_CALLER_MARKER));
        if !is_session {
            return Err(FeedAccessError::Unauthorized);
        }

        let caller_did = self.caller.as_ref().map(|c| c.caller_did.as_str()).unwrap_or("");
        let owner = self.route_handler.inner.registry.owner_of(&self.preamble.service_id);
        if owner.as_deref() != Some(caller_did) {
            return Err(FeedAccessError::Forbidden);
        }

        let Some(app_instance) = bus.app_instance_of(&self.preamble.service_id) else {
            return Err(FeedAccessError::NotAppService);
        };

        let accepts_sse = headers
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        if !accepts_sse {
            return Err(FeedAccessError::NotAcceptable);
        }

        Ok(app_instance)
    }

    pub(super) async fn handle_event_feed(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        self.handle_event_feed_parts(req.headers(), req.uri().query()).await
    }

    pub(super) async fn handle_event_feed_parts(
        &self,
        headers: &HeaderMap,
        query: Option<&str>,
    ) -> Result<Response<HttpBody>> {
        let Some(bus) = self.route_handler.inner.event_bus.as_ref() else {
            return Ok(http_error(StatusCode::NOT_FOUND, "Not Found".into()));
        };

        let app_instance = match self.check_feed_access(bus, headers) {
            Ok(inst) => inst,
            Err(err) => return Ok(err.into_response()),
        };

        let permit = match bus.try_acquire_feed_permit(&app_instance) {
            Some(p) => p,
            None => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "feed concurrency limit reached".into(),
                );
                resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                return Ok(resp);
            }
        };

        let cursor_state = parse_start_cursor(headers, query);
        let guard = FeedGuard::new();
        let changed_rx = bus.changed(&app_instance);
        let now_ms = current_time_ms();
        let (last_cursor, buffered_frames) =
            init_initial_buffered_frames(bus, &app_instance, cursor_state, now_ms);

        let session_token = self.caller.as_ref().and_then(|c| {
            c.session
                .claims
                .get("roym.session.token")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        });
        let session_exp_secs = self
            .caller
            .as_ref()
            .and_then(|c| c.session.claims.get("roym.session.exp").and_then(|v| v.as_u64()));
        let session_revocation = self.route_handler.inner.session_revocation.clone();

        if is_session_expired_or_revoked(
            session_token.as_deref(),
            session_exp_secs,
            session_revocation.as_ref(),
        ) {
            return Ok(http_error(StatusCode::UNAUTHORIZED, "Unauthorized".into()));
        }

        let state = FeedState {
            bus: bus.clone(),
            app_instance,
            last_cursor,
            buffered_frames,
            changed_rx,
            session_token,
            session_exp_secs,
            session_revocation,
            _permit: permit,
            _guard: guard,
        };

        let body = make_feed_stream(state);
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| anyhow!("failed to build SSE response: {e}"))
    }
}
