//! The counterparty's side of "agree as listed": what a tap carries, the local
//! row that lets the automatic acceptance sign later with no screen, and the
//! checks `enquiry.set` runs before it signs anything.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    as_listed::{self, AsListed, MAX_CONSENT_SECS, NotComplete, SlotChoice},
    envelope::Response,
    lang::LangTag,
    listing::{self, ListingPayload},
    method::{SiblingError, call_sibling},
    services,
    transaction::EnquiryPayload,
};

use super::super::{AS_LISTED, put_row};

/// The shortest consent a tap may ask for. Shorter than a quote's own floor,
/// which already bounds how long the owner's answer can stay valid.
pub(crate) const MIN_VALID_FOR_SECS: u64 = 60;

/// What a tap sends along with the enquiry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsListedInput {
    /// The offering version, as the search hit gave it.
    pub envelope: String,
    #[serde(default)]
    pub slot: Option<SlotChoice>,
    /// How long the person's consent holds, in seconds.
    pub valid_for_secs: u64,
    pub viewed_lang: String,
    pub viewed_via_mt: bool,
}

/// One tap, checked against the offering it names.
pub(crate) struct Resolved {
    pub(crate) listing: ListingPayload,
    /// The offering version as the hit gave it, kept whole for the automatic
    /// acceptance to verify again.
    pub(crate) envelope: String,
    pub(crate) consent: AsListed,
    pub(crate) viewed_lang: String,
    pub(crate) viewed_via_mt: bool,
}

/// The local row that remembers a tap. The envelope is kept whole so the
/// automatic acceptance can verify it again, and the viewed language is kept
/// for the acceptance half that signs with no person at the screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AsListedRow {
    pub(crate) enquiry_record_id: String,
    pub(crate) listing_id: String,
    pub(crate) conversation: String,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slot: Option<SlotChoice>,
    pub(crate) valid_until_secs: u64,
    pub(crate) viewed_lang: String,
    pub(crate) viewed_via_mt: bool,
    pub(crate) state: AsListedState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quote_record_id: Option<String>,
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AsListedState {
    /// The enquiry is signed and sent; nothing answers it yet.
    Waiting,
    /// A quote answers it, but its terms are not the offering's, so the Hub
    /// shows it as a normal quote with `reason`.
    Shown,
    /// The acceptance half is signed; the deal is agreed.
    Accepted,
    /// An answer is being signed. The quote id is fixed, so a retry reuses it.
    Answering,
    /// A quote answers the tap.
    Answered,
    /// The owner's node declined it with a reason code.
    Declined,
}

/// Runs every check a tap needs before the enquiry is signed, and builds the
/// consent it carries.
pub(crate) async fn resolve<H: AppHost>(
    host: &H,
    input: &AsListedInput,
    conversation: &str,
    owner: &str,
    now: u64,
) -> Result<Resolved, Response> {
    let verdict = listing::verify_envelope(&input.envelope, now);
    let Some(payload) = verdict.payload else {
        return Err(refusal("as-listed-not-complete", "the offering does not verify"));
    };
    let Some(issuer) = verdict.issuer else {
        return Err(refusal("as-listed-not-complete", "the offering has no issuer"));
    };
    if issuer == owner {
        return Err(refusal(
            "as-listed-own-listing",
            "this node's owner issued the offering, so the enquiry would be to itself",
        ));
    }
    let peer = conversation_peer(host, conversation).await?;
    if peer.as_deref() != Some(payload.conversation_address.as_str()) {
        return Err(refusal(
            "as-listed-wrong-conversation",
            "the offering's conversation address is not this conversation's peer",
        ));
    }
    if !(MIN_VALID_FOR_SECS..=MAX_CONSENT_SECS).contains(&input.valid_for_secs) {
        return Err(refusal("consent-window-invalid", "valid_for_secs must be 60..=604800"));
    }
    LangTag::parse_canonical(&input.viewed_lang)
        .map_err(|e| Response::invalid_params(format!("invalid viewed_lang: {e}")))?;

    let record_id = verdict
        .record_id
        .clone()
        .ok_or_else(|| refusal("as-listed-not-complete", "the offering has no record id"))?;
    let consent = AsListed {
        record_id,
        slot: input.slot.clone(),
        variant: None,
        valid_until_secs: now.saturating_add(input.valid_for_secs),
    };
    if let Err(e) = consent.validate(Some(&payload.listing_id)) {
        return Err(refusal("as-listed-not-complete", &e.to_string()));
    }
    if let Err(e) = as_listed::terms_from_listing(&payload, &issuer, &consent, now) {
        return Err(as_listed_refusal(e));
    }

    Ok(Resolved {
        listing: payload,
        envelope: input.envelope.clone(),
        consent,
        viewed_lang: input.viewed_lang.clone(),
        viewed_via_mt: input.viewed_via_mt,
    })
}

/// Stores the tap's row, keyed by the enquiry's record id. The payload is
/// read from the signed enquiry so the listing and the consent cannot drift
/// from what was signed.
pub(crate) async fn store<H: AppHost>(
    host: &H,
    payload: &EnquiryPayload,
    record_id: &str,
    resolved: Resolved,
    now: u64,
) -> Result<(), Response> {
    let consent = payload
        .as_listed
        .as_ref()
        .ok_or_else(|| Response::internal_error("an as-listed enquiry lost its consent"))?;
    let row = AsListedRow {
        enquiry_record_id: record_id.to_string(),
        listing_id: resolved.listing.listing_id,
        conversation: payload.conversation.clone(),
        envelope: resolved.envelope,
        record_id: consent.record_id.clone(),
        slot: consent.slot.clone(),
        valid_until_secs: consent.valid_until_secs,
        viewed_lang: resolved.viewed_lang,
        viewed_via_mt: resolved.viewed_via_mt,
        state: AsListedState::Waiting,
        reason: None,
        quote_record_id: None,
        created_at_secs: now,
    };
    put_row(host, AS_LISTED, record_id, &row).await.map_err(Response::internal_error)
}

/// The refusal `terms_from_listing` gave, as `enquiry.set` reports it.
fn as_listed_refusal(e: NotComplete) -> Response {
    Response::invalid_params(format!("as-listed-not-complete: {}", e.code()))
        .with_data(json!({ "code": "as-listed-not-complete", "reason": e.code() }))
}

fn refusal(code: &'static str, message: &str) -> Response {
    Response::invalid_params(message.to_string()).with_data(json!({ "code": code }))
}

/// The other party of a direct conversation, from the conversation service.
async fn conversation_peer<H: AppHost>(
    host: &H,
    conversation: &str,
) -> Result<Option<String>, Response> {
    let params = json!({ "kind": "direct", "offset": 0, "limit": 500 });
    match call_sibling::<_, serde_json::Value, ConversationListResult>(
        host,
        services::CONVERSATION,
        "conversation.list",
        &params,
    )
    .await
    {
        Ok(res) => {
            Ok(res.conversations.into_iter().find(|c| c.id == conversation).map(|c| c.peer_address))
        }
        Err(SiblingError::Refused(_) | SiblingError::Transport(_) | SiblingError::BadReply(_)) => {
            Err(Response::internal_error("conversation.list failed"))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConversationListResult {
    conversations: Vec<ConversationItem>,
}

#[derive(Debug, Deserialize)]
struct ConversationItem {
    id: String,
    peer_address: String,
}
