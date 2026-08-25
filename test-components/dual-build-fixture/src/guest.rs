#![cfg(target_arch = "wasm32")]
//! WASM wiring: `syneroym-app-host::guest::GuestHost` supplies the import
//! side (`data-layer`/`blob-store`/`messaging/host-api`); this module only
//! needs to `generate!` the export side (`test-driver`, `guest-api`,
//! `stream-types`) and remap the import interfaces onto
//! `syneroym-app-host`'s own bindings rather than regenerating them -- see
//! `syneroym-app-host`'s `lib.rs`/`guest.rs` doc comments for why a second
//! `generate!` pass over the same imports, linked into one component,
//! fails to encode.

use bindings::exports::{
    syneroym::{
        conversation::guest_api::Guest as ConversationGuestApiGuest,
        http::{
            incoming_handler::Guest as IncomingHandlerGuest,
            websocket_handler::Guest as WebSocketHandlerGuest,
        },
        messaging::{
            guest_api::Guest as GuestApiGuest,
            stream_types::{Guest as StreamTypesGuest, GuestStreamCursor, GuestStreamSink},
        },
    },
    syneroym_test::dual_build_fixture::test_driver::Guest as TestDriverGuest,
};
use syneroym_app_host::guest::{GuestHost, block_on};

/// The only place generated `unsafe` enters this crate, so the workspace's
/// `unsafe_code = "deny"` is relaxed here and nowhere else -- the crate keeps
/// every other workspace lint, including the deny-level correctness and
/// suspicious groups.
#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "dual-build-fixture",
        with: {
            "syneroym:data-layer/store@0.1.0": syneroym_wit_interfaces::data_layer::syneroym::data_layer::store,
            "syneroym:blob-store/blob-store@0.1.0": syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store,
            "syneroym:messaging/host-api@0.1.0": syneroym_wit_interfaces::messaging::syneroym::messaging::host_api,
            "syneroym:messaging/stream-types@0.1.0": generate,
            "syneroym:conversation/conversation@0.1.0": syneroym_wit_interfaces::conversation::syneroym::conversation::conversation,
            "syneroym:proxy/proxy@0.1.0": syneroym_wit_interfaces::proxy::syneroym::proxy::proxy,
            "syneroym:app-config/app-config@0.1.0": syneroym_wit_interfaces::app_config::syneroym::app_config::app_config,
            "syneroym:vault/vault@0.1.0": syneroym_wit_interfaces::vault::syneroym::vault::vault,
            "syneroym:http/websocket@0.1.0": syneroym_wit_interfaces::http_guest::syneroym::http::websocket,
            "syneroym:http/websocket-types@0.1.0": generate,
            "syneroym:http/incoming-handler@0.1.0": generate,
        },
    });
    use super::Fixture;
    export!(Fixture);
}

struct Fixture;

impl TestDriverGuest for Fixture {
    fn run(request: String) -> Result<String, String> {
        block_on(crate::app::run(&GuestHost, &request))
    }
}

impl GuestApiGuest for Fixture {
    fn handle_message(topic: String, payload: Vec<u8>) -> Result<(), String> {
        block_on(crate::app::on_message(&GuestHost, topic, payload))
    }

    fn handle_stream_request(
        _protocol: String,
        _peer_id: String,
        _request_data: Vec<u8>,
    ) -> Result<bindings::exports::syneroym::messaging::stream_types::StreamCursor, String> {
        Err("streaming not supported by this fixture".to_string())
    }

    fn accept_stream_upload(
        _protocol: String,
        _peer_id: String,
        _metadata: String,
    ) -> Result<bindings::exports::syneroym::messaging::stream_types::StreamSink, String> {
        Err("streaming not supported by this fixture".to_string())
    }
}

impl ConversationGuestApiGuest for Fixture {
    fn on_message(
        msg: bindings::exports::syneroym::conversation::guest_api::Message,
    ) -> Result<(), String> {
        block_on(crate::app::on_conversation_message(&GuestHost, msg))
    }

    fn on_delivery_state(
        msg: String,
        state: bindings::exports::syneroym::conversation::guest_api::DeliveryState,
    ) -> Result<(), String> {
        block_on(crate::app::on_conversation_state(&GuestHost, msg, state))
    }
}

fn caller_auth_in(
    auth: bindings::exports::syneroym::http::incoming_handler::CallerAuth,
) -> syneroym_app_host::types::http::CallerAuth {
    match auth {
        bindings::exports::syneroym::http::incoming_handler::CallerAuth::Delegated => {
            syneroym_app_host::types::http::CallerAuth::Delegated
        }
        bindings::exports::syneroym::http::incoming_handler::CallerAuth::Ucan => {
            syneroym_app_host::types::http::CallerAuth::Ucan
        }
        bindings::exports::syneroym::http::incoming_handler::CallerAuth::SelfAsserted => {
            syneroym_app_host::types::http::CallerAuth::SelfAsserted
        }
    }
}

fn caller_identity_in(
    id: bindings::exports::syneroym::http::incoming_handler::CallerIdentity,
) -> syneroym_app_host::types::http::CallerIdentity {
    syneroym_app_host::types::http::CallerIdentity {
        did: id.did,
        auth: caller_auth_in(id.auth),
        app_instance: id.app_instance,
    }
}

fn http_request_in(
    req: bindings::exports::syneroym::http::incoming_handler::HttpRequest,
) -> syneroym_app_host::types::http::HttpRequest {
    syneroym_app_host::types::http::HttpRequest {
        method: req.method,
        path: req.path,
        query: req.query,
        route: req.route,
        path_params: req.path_params,
        headers: req.headers,
        body: req.body,
        caller: req.caller.map(caller_identity_in),
    }
}

fn http_response_out(
    resp: syneroym_app_host::types::http::HttpResponse,
) -> bindings::exports::syneroym::http::incoming_handler::HttpResponse {
    bindings::exports::syneroym::http::incoming_handler::HttpResponse {
        status: resp.status,
        headers: resp.headers,
        body: resp.body,
    }
}

fn frame_kind_in(
    kind: bindings::exports::syneroym::http::websocket_handler::FrameKind,
) -> syneroym_app_host::types::http::FrameKind {
    match kind {
        bindings::exports::syneroym::http::websocket_handler::FrameKind::Text => {
            syneroym_app_host::types::http::FrameKind::Text
        }
        bindings::exports::syneroym::http::websocket_handler::FrameKind::Binary => {
            syneroym_app_host::types::http::FrameKind::Binary
        }
    }
}

impl IncomingHandlerGuest for Fixture {
    fn handle_request(
        request: bindings::exports::syneroym::http::incoming_handler::HttpRequest,
    ) -> Result<bindings::exports::syneroym::http::incoming_handler::HttpResponse, String> {
        let out = block_on(crate::app::handle_http(&GuestHost, http_request_in(request)))?;
        Ok(http_response_out(out))
    }
}

impl WebSocketHandlerGuest for Fixture {
    fn on_open(conn: String) {
        block_on(crate::app::on_ws_open(&GuestHost, conn));
    }

    fn on_message(
        conn: String,
        frame: Vec<u8>,
        kind: bindings::exports::syneroym::http::websocket_handler::FrameKind,
    ) {
        block_on(crate::app::on_ws_message(&GuestHost, conn, frame, frame_kind_in(kind)));
    }

    fn on_close(conn: String) {
        block_on(crate::app::on_ws_close(&GuestHost, conn));
    }
}

/// This fixture doesn't exercise raw-stream messaging, but must still
/// satisfy `guest-api`'s `use stream-types.{stream-cursor, stream-sink}`
/// reference, since `stream-types` is guest-implemented (ADR-0014). Never
/// constructed in practice: `handle-stream-request`/`accept-stream-upload`
/// above always return `Err` before any instance of these types would need
/// to exist.
pub struct UnusedStreamCursor;

impl GuestStreamCursor for UnusedStreamCursor {
    fn next_chunk(&self) -> Result<Option<Vec<u8>>, String> {
        Err("streaming not supported by this fixture".to_string())
    }
}

pub struct UnusedStreamSink;

impl GuestStreamSink for UnusedStreamSink {
    fn push_chunk(&self, _data: Vec<u8>) -> Result<(), String> {
        Err("streaming not supported by this fixture".to_string())
    }

    fn finalize(&self) -> Result<(), String> {
        Err("streaming not supported by this fixture".to_string())
    }
}

impl StreamTypesGuest for Fixture {
    type StreamCursor = UnusedStreamCursor;
    type StreamSink = UnusedStreamSink;
}
