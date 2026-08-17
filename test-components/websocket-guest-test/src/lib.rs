#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Guest WebSocket handler test component.
//!
//! Exercises `syneroym:http/websocket-handler` end to end.

use bindings::{
    exports::syneroym::http::websocket_handler::Guest as WebsocketHandlerGuest,
    syneroym::http::websocket_types::FrameKind,
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "websocket-guest-test",
        with: {
            "syneroym:http/websocket-handler@0.1.0": generate,
            "syneroym:http/websocket-types@0.1.0": generate,
            "syneroym:http/websocket@0.1.0": generate,
        },
    });

    use super::WebsocketGuestTestComponent;
    export!(WebsocketGuestTestComponent);
}

struct WebsocketGuestTestComponent;

impl WebsocketHandlerGuest for WebsocketGuestTestComponent {
    fn on_open(conn: String) {
        let _ = bindings::syneroym::http::websocket::send(&conn, b"welcome", FrameKind::Text);
    }

    fn on_message(conn: String, frame: Vec<u8>, kind: FrameKind) {
        // Echo back the frame preserving frame kind
        let _ = bindings::syneroym::http::websocket::send(&conn, &frame, kind);
    }

    fn on_close(_conn: String) {
        // Nothing to do
    }
}

impl bindings::Guest for WebsocketGuestTestComponent {
    fn init() -> Result<(), String> {
        Ok(())
    }

    fn migrate() -> Result<(), String> {
        Ok(())
    }
}
