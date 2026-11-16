//! Seat reservation and hold tracking for deals.

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, RecordWriteValue},
};
use syneroym_roym_core::{
    method::{
        SiblingError, call_sibling,
        catalog::{SlotKeyInput, SlotRow},
    },
    services,
};

use super::{LEDGER, get_row};

pub const MAX_SLOT_CAPACITY: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveOutcome {
    Held { seat: u32 },
    AlreadyHeld { seat: u32 },
    SlotUnavailable,
    SlotTaken,
}

#[allow(async_fn_in_trait)]
pub trait Reservations {
    async fn reserve<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
        slot_id: &str,
        now: u64,
    ) -> Result<ReserveOutcome, String>;

    async fn confirm<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
    ) -> Result<(), String>;

    async fn release<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
    ) -> Result<(), String>;
}

pub fn hold_id(agreement: &str, commitment_id: &str) -> String {
    format!("hold:{agreement}:{commitment_id}")
}

pub fn seat_id(slot_id: &str, n: u32) -> String {
    format!("seat:{slot_id}:{n}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HoldLedgerRow {
    pub(crate) kind: String,
    pub(crate) agreement: String,
    pub(crate) commitment_id: String,
    pub(crate) slot_id: String,
    pub(crate) seat: u32,
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SeatLedgerRow {
    pub(crate) kind: String,
    pub(crate) slot_id: String,
    pub(crate) seat: u32,
    pub(crate) agreement: String,
    pub(crate) commitment_id: String,
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SeatLedger;

impl Reservations for SeatLedger {
    async fn reserve<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
        slot_id: &str,
        now: u64,
    ) -> Result<ReserveOutcome, String> {
        let h_key = hold_id(agreement, commitment_id);
        if let Ok(Some(existing_hold)) = get_row::<HoldLedgerRow, _>(host, LEDGER, &h_key).await {
            let s_key = seat_id(&existing_hold.slot_id, existing_hold.seat);
            if let Ok(Some(existing_seat)) = get_row::<SeatLedgerRow, _>(host, LEDGER, &s_key).await
                && existing_seat.agreement == agreement
                && existing_seat.commitment_id == commitment_id
            {
                return Ok(ReserveOutcome::AlreadyHeld { seat: existing_hold.seat });
            }
        }

        let slot_row = match call_sibling::<_, SlotKeyInput, Option<SlotRow>>(
            host,
            services::CATALOG,
            "availability.get",
            &SlotKeyInput { slot_id: slot_id.to_string() },
        )
        .await
        {
            Ok(Some(s)) => s,
            Ok(None) | Err(SiblingError::Refused(_) | SiblingError::BadReply(_)) => {
                return Ok(ReserveOutcome::SlotUnavailable);
            }
            Err(SiblingError::Transport(e)) => return Err(e),
        };

        let capacity = slot_row.capacity;
        if capacity == 0 {
            return Ok(ReserveOutcome::SlotUnavailable);
        }
        let capacity = std::cmp::min(capacity, MAX_SLOT_CAPACITY);

        for n in 1..=capacity {
            let hold_row = HoldLedgerRow {
                kind: "hold".to_string(),
                agreement: agreement.to_string(),
                commitment_id: commitment_id.to_string(),
                slot_id: slot_id.to_string(),
                seat: n,
                created_at_secs: now,
            };
            let s_key = seat_id(slot_id, n);
            let seat_row = SeatLedgerRow {
                kind: "seat".to_string(),
                slot_id: slot_id.to_string(),
                seat: n,
                agreement: agreement.to_string(),
                commitment_id: commitment_id.to_string(),
                created_at_secs: now,
            };

            let rows = vec![
                RecordWriteValue {
                    id: h_key.clone(),
                    payload: serde_json::to_vec(&hold_row).map_err(|e| e.to_string())?,
                },
                RecordWriteValue {
                    id: s_key.clone(),
                    payload: serde_json::to_vec(&seat_row).map_err(|e| e.to_string())?,
                },
            ];

            match AppDataLayer::create(host, LEDGER.to_string(), rows)
                .await
                .map_err(|e| e.to_string())?
            {
                None => return Ok(ReserveOutcome::Held { seat: n }),
                Some(id) if id == h_key => {
                    return Ok(ReserveOutcome::AlreadyHeld { seat: n });
                }
                Some(_) => continue,
            }
        }

        Ok(ReserveOutcome::SlotTaken)
    }

    async fn confirm<H: AppHost>(
        &self,
        _host: &H,
        _agreement: &str,
        _commitment_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn release<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
    ) -> Result<(), String> {
        let h_key = hold_id(agreement, commitment_id);
        if let Ok(Some(hold)) = get_row::<HoldLedgerRow, _>(host, LEDGER, &h_key).await {
            let s_key = seat_id(&hold.slot_id, hold.seat);
            let mutations = vec![Mutation::Delete(h_key), Mutation::Delete(s_key)];
            AppDataLayer::batch_mutate(host, LEDGER.to_string(), mutations)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
