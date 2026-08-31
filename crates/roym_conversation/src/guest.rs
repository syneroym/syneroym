#![cfg(target_arch = "wasm32")]
//! WASM wiring: exports `api` interface.

use bindings::exports::syneroym_roym::conversation::api::Guest as ApiGuest;
use syneroym_app_host::guest::{GuestHost, block_on};
use syneroym_roym_core::dual_build;

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "conversation",
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
            "syneroym:signing/signing@0.1.0":
                syneroym_wit_interfaces::signing::syneroym::signing::signing,
            "syneroym:http/websocket@0.1.0":
                syneroym_wit_interfaces::http_guest::syneroym::http::websocket,
            "syneroym:http/websocket-types@0.1.0": generate,
        },
    });
    use super::Conversation;
    export!(Conversation);
}

struct Conversation;

impl ApiGuest for Conversation {
    fn invoke(request: String) -> Result<String, String> {
        block_on(dual_build::handle_invoke(&GuestHost, &request, crate::app::invoke))
    }

    fn status() -> Result<String, String> {
        block_on(crate::app::status(&GuestHost))
    }
}
