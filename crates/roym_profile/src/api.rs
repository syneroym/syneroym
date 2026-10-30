//! Method dispatch table for the profile service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    method::{
        MethodInfo, NoParams, PingOutput, Reply,
        profile::{
            AdmitFirstContactInput, AdmitFirstContactOutput, BlockCheckOutput, BlockTargetInput,
            ContactKeyInput, ContactRow, ContactsListInput, ProfileGetInput, ProfileRecord,
            ResolveAddressOutput,
        },
    },
    safety::ContactLimits,
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
};

use crate::app::{backup, contacts, moderation, profile_ops};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::PROFILE.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "profile.ping": ping [NoParams => PingOutput],
    "profile.policy": profile_ops::policy [NoParams => profile_ops::PolicyOutput],
    "profile.get": profile_ops::get [ProfileGetInput => Option<ProfileRecord>],
    "profile.set": profile_ops::set [profile_ops::ProfileSetInput => profile_ops::ProfileSetOutput],
    "profile.export": backup::export [NoParams => Bundle],
    "profile.import": backup::import [ImportInput => backup::ProfileImportOutput],
    "contacts.list": contacts::list [ContactsListInput => Vec<ContactRow>],
    "contacts.get": contacts::get [ContactKeyInput => Option<ContactRow>],
    "contacts.upsert": contacts::upsert [contacts::ContactUpsertInput => contacts::ContactUpsertOutput],
    "contacts.remove": contacts::remove [ContactKeyInput => contacts::ContactRemoveOutput],
    "contacts.resolve-address": contacts::resolve_address [ContactKeyInput => ResolveAddressOutput],
    "contacts.admit-first-contact": contacts::admit_first_contact [AdmitFirstContactInput => AdmitFirstContactOutput],
    "contacts.limits": contacts::limits [NoParams => ContactLimits],
    "contacts.set-limits": contacts::set_limits [contacts::ContactLimitsInput => ContactLimits],
    "block.add": moderation::block_add [moderation::BlockAddInput => moderation::BlockAddOutput],
    "block.remove": moderation::block_remove [BlockTargetInput => moderation::BlockRemoveOutput],
    "block.list": moderation::block_list [moderation::BlockListInput => Vec<moderation::BlockRow>],
    "block.check": moderation::block_check [BlockTargetInput => BlockCheckOutput],
    "report.create": moderation::report_create [moderation::ReportCreateInput => moderation::ReportCreateOutput],
    "report.list": moderation::report_list [moderation::ReportListInput => Vec<moderation::ReportRow>],
    "report.get": moderation::report_get [moderation::ReportIdInput => Option<moderation::ReportRow>],
    "report.withdraw": moderation::report_withdraw [moderation::ReportIdInput => moderation::ReportCreateOutput],
    "profile.signing-status": signing_status [NoParams => SigningStatusOutput],
    "profile.install-signing-certificate": install_signing_certificate [InstallCertificateInput => InstallCertificateOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
