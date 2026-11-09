use thiserror::Error;

use super::{
    MAX_ADDRESS_LEN, MAX_CATEGORIES, MAX_CATEGORY_LEN, MAX_DESCRIPTION_LEN, MAX_NOTICE_LEN,
    MAX_PAYEE_LEN, MAX_PAYMENT_METHOD_LEN, MAX_PAYMENT_METHODS, MAX_SCOPE_LEN, MAX_TERMS_TEXT_LEN,
};
use crate::area::AreaError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransactionError {
    #[error("latest_secs ({latest_secs}) is before earliest_secs ({earliest_secs})")]
    InvalidTimeWindow { earliest_secs: u64, latest_secs: u64 },

    #[error("address is only applicable when location is at-receiver")]
    AddressNotApplicable,

    #[error("address is over {MAX_ADDRESS_LEN} characters")]
    AddressTooLong,

    #[error("area: {0}")]
    Area(#[from] AreaError),

    #[error("scope is empty")]
    ScopeEmpty,

    #[error("scope is over {MAX_SCOPE_LEN} characters")]
    ScopeTooLong,

    #[error("currency '{0}' is not a currency code this build knows")]
    CurrencyUnknown(String),

    #[error("amount_minor ({0}) cannot be negative")]
    NegativeAmount(i64),

    #[error("tax_minor ({0}) cannot be negative")]
    NegativeTax(i64),

    #[error("fees_minor ({0}) cannot be negative")]
    NegativeFees(i64),

    #[error(
        "tax_minor ({tax_minor}) + fees_minor ({fees_minor}) exceeds amount_minor ({amount_minor})"
    )]
    BreakdownExceedsTotal { tax_minor: i64, fees_minor: i64, amount_minor: i64 },

    #[error("payee is empty")]
    PayeeEmpty,

    #[error("payee is over {MAX_PAYEE_LEN} characters")]
    PayeeTooLong,

    #[error("more than {MAX_PAYMENT_METHODS} payment methods")]
    TooManyPaymentMethods,

    #[error("payment method '{0}' is not 1..={MAX_PAYMENT_METHOD_LEN} chars of [a-z0-9-]")]
    PaymentMethodShape(String),

    #[error("cancellation_terms is empty")]
    CancellationTermsEmpty,

    #[error("cancellation_terms is over {MAX_TERMS_TEXT_LEN} characters")]
    CancellationTermsTooLong,

    #[error("refund_terms is empty")]
    RefundTermsEmpty,

    #[error("refund_terms is over {MAX_TERMS_TEXT_LEN} characters")]
    RefundTermsTooLong,

    #[error("dispute_path is empty")]
    DisputePathEmpty,

    #[error("dispute_path is over {MAX_TERMS_TEXT_LEN} characters")]
    DisputePathTooLong,

    #[error("quote_expires_at_secs must be non-zero")]
    QuoteExpiryZero,

    #[error("quote_id is empty")]
    QuoteIdEmpty,

    #[error("conversation is empty")]
    ConversationEmpty,

    #[error("sequence must be >= 1, got {0}")]
    InvalidSequence(u32),

    #[error("record_id '{0}' must start with 'rec_'")]
    InvalidRecordId(String),

    #[error("did '{0}' is not a valid did:key")]
    InvalidDid(String),

    #[error("counterparty and quote author cannot be the same DID")]
    SamePartyDid,

    #[error("request_id is empty")]
    RequestIdEmpty,

    #[error("description is empty")]
    DescriptionEmpty,

    #[error("description is over {MAX_DESCRIPTION_LEN} characters")]
    DescriptionTooLong,

    #[error("more than {MAX_CATEGORIES} categories")]
    TooManyCategories,

    #[error("category '{0}' is not 1..={MAX_CATEGORY_LEN} chars of [a-z0-9-]")]
    InvalidCategory(String),

    #[error("data_use_notice is empty")]
    DataUseNoticeEmpty,

    #[error("data_use_notice is over {MAX_NOTICE_LEN} characters")]
    DataUseNoticeTooLong,

    #[error("slot_id requires both listing_id and terms.schedule")]
    SlotNeedsListingAndSchedule,

    #[error("slot_id does not match the id derived from listing_id and terms.schedule")]
    SlotIdMismatch,
}
