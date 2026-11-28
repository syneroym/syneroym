//! Forwarding a checked, signed text message to another chat.
//!
//! A forward copies the author's text, mode and origin byte for byte. It
//! signs nothing: the forwarder's own message signature covers the whole
//! body, so a person with no signing certificate can forward.

use serde_json::json;
use syneroym_app_host::{AppConversation, AppHost, types::conversation::ConversationError};
use syneroym_roym_core::{
    chat::{
        self, CHAT_BODY_VERSION, ChatBody, ForwardMode, OriginCheck, ROYM_MESSAGE_CONTENT_TYPE,
    },
    clock,
    envelope::Response,
    method::{
        Reply,
        conversation::{ForwardInput, SendOutput},
    },
};

use super::{from_host, messages};

pub(super) fn refuse(code: &str, message: impl Into<String>) -> Response {
    Response::invalid_params(message).with_data(json!({ "code": code }))
}

pub(crate) async fn forward<H: AppHost>(host: &H, input: ForwardInput) -> Reply<SendOutput> {
    let msg = match AppConversation::get_message(host, input.message_id).await {
        Ok(m) => m,
        Err(ConversationError::NotFound) => {
            return Err(refuse("message-not-found", "the message was not found"));
        }
        Err(e) => return Err(from_host(e)),
    };

    if msg.deleted_at.is_some() {
        return Err(refuse("message-deleted", "the message was deleted"));
    }
    if msg.content_type != ROYM_MESSAGE_CONTENT_TYPE {
        return Err(refuse("not-forwardable", "this message is not forwardable"));
    }
    let Ok(body) = ChatBody::parse(&msg.body) else {
        return Err(refuse("not-forwardable", "the message body could not be read"));
    };
    if body.forward_mode != ForwardMode::Signed {
        return Err(refuse("not-forwardable", "the author did not allow forwarding"));
    }
    let now = clock::now_secs();
    let check = chat::check_origin(&body, &msg.languages, now);
    let OriginCheck::Checked { .. } = &check else {
        let reason = match check {
            OriginCheck::Failed { reason } => reason,
            _ => "no signed origin".to_string(),
        };
        return Err(refuse("origin-not-checked", reason));
    };

    let old = body.forward_count.unwrap_or(0);
    let new_count = chat::next_forward_count(old, msg.outgoing && old >= 1);
    let forwarded = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: body.content_type,
        text: body.text,
        forward_mode: ForwardMode::Signed,
        origin: body.origin,
        forward_count: Some(new_count),
    };

    messages::send_body(
        host,
        input.conversation,
        ROYM_MESSAGE_CONTENT_TYPE,
        msg.languages,
        forwarded.to_bytes(),
        |_, _| None,
    )
    .await
}
