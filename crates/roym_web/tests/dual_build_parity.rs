#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration suite driving the Roym SynApp through both builds -- the real
//! `wasm32-wasip2` components via `AppSandboxEngine`, and the same sources
//! linked in via `syneroym-app-host-native` -- and asserting the results
//! are identical across all scenarios.

#[path = "dual_build_parity/deal_fixtures.rs"]
pub(crate) mod deal_fixtures;
#[path = "dual_build_parity/fixtures.rs"]
pub(crate) mod fixtures;
#[path = "dual_build_parity/helpers.rs"]
pub(crate) mod helpers;
#[path = "dual_build_parity/trust_fixtures.rs"]
pub(crate) mod trust_fixtures;
#[path = "dual_build_parity/trust_harness.rs"]
pub(crate) mod trust_harness;

#[path = "dual_build_parity/admission.rs"]
mod admission;
#[path = "dual_build_parity/agreement_lang.rs"]
mod agreement_lang;
#[path = "dual_build_parity/as_listed.rs"]
mod as_listed;
#[path = "dual_build_parity/booking.rs"]
mod booking;
#[path = "dual_build_parity/bundles.rs"]
mod bundles;
#[path = "dual_build_parity/card_sync.rs"]
mod card_sync;
#[path = "dual_build_parity/catalog.rs"]
mod catalog;
#[path = "dual_build_parity/commitment_failure.rs"]
mod commitment_failure;
#[path = "dual_build_parity/conversation.rs"]
mod conversation;
#[path = "dual_build_parity/deal.rs"]
mod deal;
#[path = "dual_build_parity/directory.rs"]
mod directory;
#[path = "dual_build_parity/directory_compat.rs"]
mod directory_compat;
#[path = "dual_build_parity/directory_extension.rs"]
mod directory_extension;
#[path = "dual_build_parity/directory_find.rs"]
mod directory_find;
#[path = "dual_build_parity/disclosure.rs"]
mod disclosure;
#[path = "dual_build_parity/events.rs"]
mod events;
#[path = "dual_build_parity/forward.rs"]
mod forward;
#[path = "dual_build_parity/fulfilment.rs"]
mod fulfilment;
#[path = "dual_build_parity/group.rs"]
mod group;
#[path = "dual_build_parity/group_lifecycle.rs"]
mod group_lifecycle;
#[path = "dual_build_parity/payment.rs"]
mod payment;
#[path = "dual_build_parity/profile.rs"]
mod profile;
#[path = "dual_build_parity/scheduling.rs"]
mod scheduling;
#[path = "dual_build_parity/search_scripts.rs"]
mod search_scripts;
#[path = "dual_build_parity/statement.rs"]
mod statement;
#[path = "dual_build_parity/step.rs"]
mod step;
#[path = "dual_build_parity/transaction.rs"]
mod transaction;
#[path = "dual_build_parity/transaction_cards.rs"]
mod transaction_cards;
#[path = "dual_build_parity/transaction_validation.rs"]
mod transaction_validation;
#[path = "dual_build_parity/translation.rs"]
mod translation;
#[path = "dual_build_parity/trust.rs"]
mod trust;
#[path = "dual_build_parity/trust_evidence_cap.rs"]
mod trust_evidence_cap;
#[path = "dual_build_parity/trust_sources.rs"]
mod trust_sources;
#[path = "dual_build_parity/widget.rs"]
mod widget;
#[path = "dual_build_parity/wire_origin.rs"]
mod wire_origin;
