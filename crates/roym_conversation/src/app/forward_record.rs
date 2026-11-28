//! Forwarding a signed Roym record to another chat.
//!
//! The record travels as itself: this body carries the envelope and the name
//! of its type. This service does not check the record. A client asks the
//! service that owns the type to check it before it draws it, so the check
//! stays where it is today and this service links no record payload code.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    method::{
        Reply,
        conversation::{ForwardRecordInput, SendOutput},
    },
    record_forward::{RECORD_FORWARD_CONTENT_TYPE, RecordForwardBody, RecordForwardError},
};

use super::{forward, messages};

fn code_for(error: &RecordForwardError) -> &'static str {
    match error {
        RecordForwardError::TooLarge => "record-too-large",
        RecordForwardError::UnsupportedVersion(_) => "record-version-unsupported",
        RecordForwardError::UnsupportedRecordType(_) => "record-type-not-supported",
        RecordForwardError::Malformed(_) => "record-not-readable",
    }
}

pub(crate) async fn forward_record<H: AppHost>(
    host: &H,
    input: ForwardRecordInput,
) -> Reply<SendOutput> {
    let body = match RecordForwardBody::new(&input.record_type, &input.envelope) {
        Ok(body) => body,
        Err(e) => return Err(forward::refuse(code_for(&e), e.to_string())),
    };
    let bytes = match body.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => return Err(forward::refuse(code_for(&e), e.to_string())),
    };
    messages::send_body(
        host,
        input.conversation,
        RECORD_FORWARD_CONTENT_TYPE,
        Vec::new(),
        bytes,
        |_, _| None,
    )
    .await
}
