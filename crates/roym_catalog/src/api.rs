//! Method dispatch table for the catalog service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    listing::ListingVerdict,
    method::{
        EnvelopeHistoryOutput, MethodInfo, NoParams, PingOutput, Reply, VerifyInput,
        catalog::{ListingGetOutput, ListingKeyInput, SlotKeyInput, SlotRow},
    },
    safety::PublicationLimits,
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
};

use crate::app::{availability, backup, limits, listing_ops};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::CATALOG.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "listing.ping": ping [NoParams => PingOutput],
    "listing.set": listing_ops::set [listing_ops::SetListingInput => listing_ops::ListingWriteOutput],
    "listing.withdraw": listing_ops::withdraw [ListingKeyInput => listing_ops::ListingWriteOutput],
    "listing.get": listing_ops::get [ListingKeyInput => Option<ListingGetOutput>],
    "listing.list": listing_ops::list [listing_ops::ListingListInput => listing_ops::ListingListOutput],
    "listing.history": listing_ops::history [ListingKeyInput => EnvelopeHistoryOutput],
    "listing.verify": listing_ops::verify [VerifyInput => ListingVerdict],
    "listing.limits": limits::limits [NoParams => PublicationLimits],
    "listing.set-limits": limits::set_limits [limits::PublicationLimitsInput => PublicationLimits],
    "availability.set": availability::set [availability::AvailabilitySetInput => availability::AvailabilitySetOutput],
    "availability.get": availability::get [SlotKeyInput => Option<SlotRow>],
    "availability.list": availability::list [availability::AvailabilityListInput => availability::AvailabilityListOutput],
    "availability.remove": availability::remove [SlotKeyInput => availability::AvailabilityRemoveOutput],
    "catalog.export": backup::export [NoParams => Bundle],
    "catalog.import": backup::import [ImportInput => backup::CatalogImportOutput],
    "catalog.signing-status": signing_status [NoParams => SigningStatusOutput],
    "catalog.install-signing-certificate": install_signing_certificate [InstallCertificateInput => InstallCertificateOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
