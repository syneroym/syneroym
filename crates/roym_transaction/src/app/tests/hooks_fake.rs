//! A stand-in for the scheduling service: the four reservation hooks over an
//! in-memory seat map.
//!
//! The deal core's own tests use it so that they run the real hook client
//! (`ExtensionHooks`) without the real service. The seat rules are tested in
//! the scheduling crate, and in the two-node and parity tests, not twice. A
//! slot that a test does not set up is a slot of `listing_123` with one seat.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use syneroym_app_host::types::proxy::ProxyError;
use syneroym_roym_core::testing::TestHost;

pub(crate) const FAKE_LISTING: &str = "listing_123";
pub(crate) const FAKE_START: u64 = 1_000_000;
pub(crate) const FAKE_END: u64 = 1_003_600;
/// Every slot of the fake has one seat.
const SEATS_PER_SLOT: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Held,
    Confirmed,
    Free,
}

#[derive(Debug, Clone)]
struct Reservation {
    agreement: String,
    commitment_id: String,
    slot_id: String,
    status: Status,
}

/// One call that the deal core made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) method: String,
    pub(crate) agreement: Option<String>,
    pub(crate) commitment_id: Option<String>,
}

#[derive(Default)]
struct State {
    missing: HashSet<String>,
    reservations: Vec<Reservation>,
    calls: Vec<Call>,
    refuse_confirms: Option<String>,
    refuse_reserves: Option<String>,
    fail_nth: HashMap<String, usize>,
    failure_hit: bool,
}

/// Shared with the handlers that `install` registers on the host.
#[derive(Clone, Default)]
pub(crate) struct FakeScheduling {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for FakeScheduling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeScheduling").finish_non_exhaustive()
    }
}

fn refusal(code: &str) -> Result<String, ProxyError> {
    Ok(json!({ "error": { "code": -32602, "message": code, "data": { "code": code } } })
        .to_string())
}

fn text<'a>(params: &'a Value, key: &str) -> &'a str {
    params.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl FakeScheduling {
    /// Registers the four hooks on `host`.
    pub(crate) fn install(host: &TestHost) -> Self {
        let fake = Self::default();
        for method in [
            "reservation.prepare",
            "reservation.reserve",
            "reservation.confirm",
            "reservation.release",
        ] {
            let me = fake.clone();
            host.on_call("scheduling", method, move |params| me.handle(method, &params));
        }
        fake
    }

    fn handle(&self, method: &str, params: &str) -> Result<String, ProxyError> {
        let params: Value = serde_json::from_str(params).unwrap_or(Value::Null);
        let mut state = self.state.lock().unwrap();
        let agreement = params.get("agreement").and_then(Value::as_str).map(str::to_string);
        let commitment_id = params.get("commitment_id").and_then(Value::as_str).map(str::to_string);
        let seen = state.calls.iter().filter(|c| c.method == method).count();
        state.calls.push(Call { method: method.to_string(), agreement, commitment_id });
        if state.fail_nth.get(method) == Some(&seen) {
            state.failure_hit = true;
            return Err(ProxyError::Transport("injected hook failure".to_string()));
        }
        match method.strip_prefix("reservation.").unwrap_or_default() {
            "prepare" => state.prepare(&params),
            "reserve" => state.reserve(&params),
            "confirm" => state.confirm(&params),
            _ => state.release(&params),
        }
    }

    /// Makes the slot unknown: `prepare` and `reserve` refuse it.
    pub(crate) fn remove_slot(&self, slot_id: &str) {
        self.state.lock().unwrap().missing.insert(slot_id.to_string());
    }

    /// Every `confirm` is refused with `code`.
    pub(crate) fn refuse_confirms(&self, code: &str) {
        self.state.lock().unwrap().refuse_confirms = Some(code.to_string());
    }

    /// Every `reserve` is refused with `code`.
    pub(crate) fn refuse_reserves(&self, code: &str) {
        self.state.lock().unwrap().refuse_reserves = Some(code.to_string());
    }

    /// The `nth` call (from 0) of `method` fails with a transport error, once.
    pub(crate) fn fail_call(&self, method: &str, nth: usize) {
        self.state.lock().unwrap().fail_nth.insert(method.to_string(), nth);
    }

    pub(crate) fn failure_hit(&self) -> bool {
        self.state.lock().unwrap().failure_hit
    }

    /// Another agreement already holds a seat of the slot.
    pub(crate) fn take_seat(&self, slot_id: &str, agreement: &str) {
        self.state.lock().unwrap().reservations.push(Reservation {
            agreement: agreement.to_string(),
            commitment_id: "c1".to_string(),
            slot_id: slot_id.to_string(),
            status: Status::Held,
        });
    }

    fn find(&self, agreement: &str, commitment_id: &str) -> Option<Status> {
        let state = self.state.lock().unwrap();
        state
            .reservations
            .iter()
            .rev()
            .find(|r| r.agreement == agreement && r.commitment_id == commitment_id)
            .map(|r| r.status)
    }

    /// Whether the reservation exists and has not been released.
    pub(crate) fn is_held(&self, agreement: &str, commitment_id: &str) -> bool {
        matches!(self.find(agreement, commitment_id), Some(Status::Held | Status::Confirmed))
    }

    pub(crate) fn is_confirmed(&self, agreement: &str, commitment_id: &str) -> bool {
        self.find(agreement, commitment_id) == Some(Status::Confirmed)
    }

    /// Whether the reservation was taken and then released.
    pub(crate) fn is_released(&self, agreement: &str, commitment_id: &str) -> bool {
        self.find(agreement, commitment_id) == Some(Status::Free)
    }

    /// How many seats of the slot are held.
    pub(crate) fn held_seats(&self, slot_id: &str) -> usize {
        let state = self.state.lock().unwrap();
        state
            .reservations
            .iter()
            .filter(|r| r.slot_id == slot_id && r.status != Status::Free)
            .count()
    }

    /// How many times the deal core called `method`.
    pub(crate) fn calls(&self, method: &str) -> usize {
        self.state.lock().unwrap().calls.iter().filter(|c| c.method == method).count()
    }

    pub(crate) fn call_log(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl State {
    fn slot_id(params: &Value) -> String {
        params
            .get("details")
            .and_then(|d| d.get("data"))
            .map(|d| text(d, "slot_id").to_string())
            .unwrap_or_default()
    }

    fn prepare(&self, params: &Value) -> Result<String, ProxyError> {
        let slot_id = Self::slot_id(params);
        if self.missing.contains(&slot_id) {
            return refusal("no-such-slot");
        }
        let data = params.get("details").and_then(|d| d.get("data")).unwrap_or(&Value::Null);
        let listing = params
            .get("listing_id")
            .and_then(Value::as_str)
            .or_else(|| data.get("listing_id").and_then(Value::as_str))
            .unwrap_or_default();
        if listing != FAKE_LISTING {
            return refusal("slot-not-in-listing");
        }
        if let Some(window) = params.get("window").filter(|w| !w.is_null())
            && (window["earliest_secs"] != FAKE_START || window["latest_secs"] != FAKE_END)
        {
            return refusal("schedule-differs-from-slot");
        }
        Ok(json!({ "window": { "earliest_secs": FAKE_START, "latest_secs": FAKE_END } })
            .to_string())
    }

    fn view(r: &Reservation) -> Value {
        let state = if r.status == Status::Confirmed { "confirmed" } else { "held" };
        json!({
            "agreement": r.agreement,
            "commitment_id": r.commitment_id,
            "state": state,
            "expires_at_secs": null,
        })
    }

    fn reserve(&mut self, params: &Value) -> Result<String, ProxyError> {
        if let Some(code) = self.refuse_reserves.clone() {
            return refusal(&code);
        }
        let (agreement, cid) = (text(params, "agreement"), text(params, "commitment_id"));
        let slot_id = Self::slot_id(params);
        if let Some(r) = self
            .reservations
            .iter()
            .rev()
            .find(|r| r.agreement == agreement && r.commitment_id == cid)
            && r.status != Status::Free
        {
            return Ok(Self::view(r).to_string());
        }
        if self.missing.contains(&slot_id) {
            return refusal("slot-unavailable");
        }
        let held = self
            .reservations
            .iter()
            .filter(|r| r.slot_id == slot_id && r.status != Status::Free)
            .count();
        if held >= SEATS_PER_SLOT {
            return refusal("slot-taken");
        }
        let r = Reservation {
            agreement: agreement.to_string(),
            commitment_id: cid.to_string(),
            slot_id,
            status: Status::Held,
        };
        let view = Self::view(&r);
        self.reservations.push(r);
        Ok(view.to_string())
    }

    fn confirm(&mut self, params: &Value) -> Result<String, ProxyError> {
        if let Some(code) = self.refuse_confirms.clone() {
            return refusal(&code);
        }
        let (agreement, cid) = (text(params, "agreement"), text(params, "commitment_id"));
        let Some(r) = self
            .reservations
            .iter_mut()
            .rev()
            .find(|r| r.agreement == agreement && r.commitment_id == cid)
        else {
            return refusal("reservation-unknown");
        };
        // A released key is refused, as the real service does: a confirm
        // never takes a seat.
        if r.status == Status::Free {
            return refusal("reservation-lost");
        }
        r.status = Status::Confirmed;
        Ok(Self::view(r).to_string())
    }

    fn release(&mut self, params: &Value) -> Result<String, ProxyError> {
        let (agreement, cid) = (text(params, "agreement"), text(params, "commitment_id"));
        let found = self.reservations.iter_mut().rev().find(|r| {
            r.agreement == agreement && r.commitment_id == cid && r.status != Status::Free
        });
        let released = found.is_some();
        if let Some(r) = found {
            r.status = Status::Free;
        }
        Ok(json!({ "released": released }).to_string())
    }
}
