//! The read side of a chat body: the inner text and the forward view.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    chat::{ChatBody, ROYM_MESSAGE_CONTENT_TYPE, forward_view, unreadable_view},
    clock,
    conversation::{BodyEncoding, Direction, MessageView},
    signing,
};

use super::contacts_map;

/// Replaces the body of every chat body on the page with its inner text,
/// and records what the origin check found. The readers' own contacts are
/// read once, and only when the page holds a chat body.
pub(crate) async fn add_forward_views<H: AppHost>(host: &H, views: &mut [MessageView]) {
    if !views.iter().any(is_chat_body) {
        return;
    }
    let reader_did = signing::owner_did(host).await.ok();
    let contacts = contacts_map(host).await;
    let now = clock::now_secs();

    for view in views.iter_mut().filter(|v| is_chat_body(v)) {
        let raw = match (&view.body, view.body_encoding) {
            (Some(raw), BodyEncoding::Utf8) => raw.as_str(),
            _ => {
                view.body = None;
                view.forward = Some(unreadable_view("message body could not be read"));
                continue;
            }
        };
        match ChatBody::parse(raw.as_bytes()) {
            Ok(body) => {
                let sender = if view.direction == Direction::Outgoing {
                    reader_did.clone()
                } else {
                    contacts.get(&view.author).cloned()
                };
                view.body = Some(body.text.clone());
                view.forward = Some(forward_view(
                    &body,
                    &view.languages,
                    sender.as_deref(),
                    reader_did.as_deref(),
                    now,
                ));
            }
            Err(_) => {
                view.body = None;
                view.forward = Some(unreadable_view("message body could not be read"));
            }
        }
    }
}

fn is_chat_body(view: &MessageView) -> bool {
    view.content_type == ROYM_MESSAGE_CONTENT_TYPE && !view.is_deleted()
}
