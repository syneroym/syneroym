//! Method dispatch table for the scheduling service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    method::{
        MethodInfo, NoParams, PingOutput, Reply,
        scheduling::{
            PrepareInput, PrepareOutput, ReleaseOutput, ReservationKey, ReservationView,
            ReserveInput, SlotKeyInput, SlotRow,
        },
    },
    services,
};

use crate::app::{availability, backup, prepare, reservation};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::SCHEDULING.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "scheduling.ping": ping [NoParams => PingOutput],
    "availability.set": availability::set [availability::AvailabilitySetInput => availability::AvailabilitySetOutput],
    "availability.get": availability::get [SlotKeyInput => Option<SlotRow>],
    "availability.list": availability::list [availability::AvailabilityListInput => availability::AvailabilityListOutput],
    "availability.remove": availability::remove [SlotKeyInput => availability::AvailabilityRemoveOutput],
    "reservation.prepare": prepare::prepare [PrepareInput => PrepareOutput],
    "reservation.reserve": reservation::reserve [ReserveInput => ReservationView],
    "reservation.confirm": reservation::confirm [ReservationKey => ReservationView],
    "reservation.release": reservation::release [ReservationKey => ReleaseOutput],
    "scheduling.export": backup::export [NoParams => Bundle],
    "scheduling.import": backup::import [ImportInput => backup::SchedulingImportOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
