//! Sending a message that names a forward mode.
//!
//! The group checks run before the signing call, so a send to a group the
//! person has left signs nothing. The message signature of the send covers
//! the whole chat body, so the mode cannot be changed later.

use syneroym_app_host::{AppHost, AppSigning};
use syneroym_roym_core::{
    chat::{
        self, CHAT_BODY_VERSION, ChatBody, ForwardMode, INNER_TEXT_CONTENT_TYPE,
        MessageOriginPayload, ROYM_MESSAGE_CONTENT_TYPE,
    },
    clock,
    envelope::Response,
    method::{
        ParamError, Reply,
        conversation::{SendInput, SendOutput},
    },
    signing,
};

use super::messages;

async fn sign_origin<H: AppHost>(
    host: &H,
    languages: &[String],
    text: &str,
) -> Result<String, Response> {
    let payload = MessageOriginPayload {
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        languages: languages.to_vec(),
        content_hash: chat::content_hash(INNER_TEXT_CONTENT_TYPE, languages, text.as_bytes(), &[]),
    };
    let now = clock::now_secs();
    let (principal, _owner) = signing::resolve_principal_and_owner(host, now).await?;
    AppSigning::sign_record(host, chat::origin_draft(&payload), principal)
        .await
        .map_err(|e| Response::internal_error(e.to_string()))
}

pub(crate) async fn send_with_mode<H: AppHost>(
    host: &H,
    input: SendInput,
    mode: ForwardMode,
) -> Reply<SendOutput> {
    if let Some(ct) = input.content_type.as_deref()
        && ct != INNER_TEXT_CONTENT_TYPE
    {
        return Err(ParamError {
            field: Some("content_type".into()),
            message: "a message with a forward mode is text/plain".into(),
        }
        .into_response());
    }
    let languages: Vec<String> = input.languages.iter().map(ToString::to_string).collect();

    messages::check_target(host, &input.conversation, ROYM_MESSAGE_CONTENT_TYPE, |_, _| None)
        .await?;

    let origin = if mode == ForwardMode::Signed {
        Some(sign_origin(host, &languages, &input.body).await?)
    } else {
        None
    };

    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: input.body,
        forward_mode: mode,
        origin,
        forward_count: None,
    };
    messages::deliver(
        host,
        input.conversation,
        ROYM_MESSAGE_CONTENT_TYPE,
        languages,
        body.to_bytes(),
    )
    .await
}
