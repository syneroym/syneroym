#![cfg(target_arch = "wasm32")]
//! WASM wiring for Web entrypoint: exports `incoming-handler`,
//! `websocket-handler`, and `api`.

use bindings::exports::{
    syneroym::http::{
        incoming_handler::Guest as IncomingHandlerGuest,
        websocket_handler::Guest as WebSocketHandlerGuest,
    },
    syneroym_roym::web::api::Guest as ApiGuest,
};
use syneroym_app_host::guest::{GuestHost, block_on};

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "web",
        with: {
            "syneroym:data-layer/store@0.1.0":
                syneroym_wit_interfaces::data_layer::syneroym::data_layer::store,
            "syneroym:blob-store/blob-store@0.1.0":
                syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store,
            "syneroym:messaging/host-api@0.1.0":
                syneroym_wit_interfaces::messaging::syneroym::messaging::host_api,
            "syneroym:conversation/conversation@0.1.0":
                syneroym_wit_interfaces::conversation::syneroym::conversation::conversation,
            "syneroym:proxy/proxy@0.1.0":
                syneroym_wit_interfaces::proxy::syneroym::proxy::proxy,
            "syneroym:app-config/app-config@0.1.0":
                syneroym_wit_interfaces::app_config::syneroym::app_config::app_config,
            "syneroym:vault/vault@0.1.0":
                syneroym_wit_interfaces::vault::syneroym::vault::vault,
            "syneroym:http/websocket@0.1.0":
                syneroym_wit_interfaces::http_guest::syneroym::http::websocket,
            "syneroym:http/websocket-types@0.1.0": generate,
            "syneroym:http/incoming-handler@0.1.0": generate,
        },
    });
    use super::Web;
    export!(Web);
}

struct Web;

impl ApiGuest for Web {
    fn invoke(request: String) -> Result<String, String> {
        block_on(syneroym_roym_core::dual_build::handle_invoke(
            &GuestHost,
            &request,
            crate::app::invoke,
        ))
    }

    fn status() -> Result<String, String> {
        block_on(crate::app::status(&GuestHost))
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

impl IncomingHandlerGuest for Web {
    fn handle_request(
        request: bindings::exports::syneroym::http::incoming_handler::HttpRequest,
    ) -> Result<bindings::exports::syneroym::http::incoming_handler::HttpResponse, String> {
        let out = block_on(crate::app::handle_http(&GuestHost, http_request_in(request)))?;
        Ok(http_response_out(out))
    }
}

impl WebSocketHandlerGuest for Web {
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
