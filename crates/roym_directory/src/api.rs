//! Method dispatch table for the directory service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    admit::WireRule,
    backup::{Bundle, ImportInput},
    directory::{Member, SearchQuery, SynOrgSettings},
    method::{
        MethodInfo, NoParams, PingOutput, PublicationLimitsInput, Reply, WireNoParams,
        catalog::ListingKeyInput,
        directory::{
            DirectoryInfoOutput, PublishInput, PublishOutput, SearchOutput, StandingInput,
            StandingOutput,
        },
    },
    safety::PublicationLimits,
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
};

use crate::app::{
    self, backup, client_merge, client_query, client_sources, credential_ops, held, moderation_ops,
    publication_ops, search_ops, standing, synorg,
};

/// The four methods a foreign node may reach. `directory.search`,
/// `directory.info` and `directory.standing` admit a stranger with no
/// identity at all -- reading something this installation publishes on
/// purpose (including a member's own signed evidence) costs nothing to
/// leave open. `directory.publish` records who asked, because a
/// publication is durable and must name a party.
pub const WIRE_REACHABLE: &[(&str, WireRule)] = &[
    ("directory.search", WireRule::Open),
    ("directory.info", WireRule::Open),
    ("directory.publish", WireRule::VerifiedOnly),
    ("directory.standing", WireRule::Open),
];

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::DIRECTORY.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "directory.ping": ping [NoParams => PingOutput],
    "directory.settings": synorg::get_settings [NoParams => Option<SynOrgSettings>],
    "directory.set-settings": synorg::set_settings [synorg::SynOrgSettingsInput => SynOrgSettings],
    "directory.info": synorg::info [WireNoParams => Option<DirectoryInfoOutput>],
    "member.add": synorg::member_add [synorg::MemberAddInput => Member],
    "member.remove": synorg::member_remove [app::DidInput => app::RemovedOutput],
    "member.list": synorg::member_list [NoParams => synorg::MemberListOutput],
    "directory.publish": publication_ops::publish [PublishInput => PublishOutput],
    "directory.unpublish": publication_ops::unpublish [ListingKeyInput => publication_ops::UnpublishOutput],
    "directory.publications": publication_ops::publications [NoParams => publication_ops::PublicationsOutput],
    "directory.search": search_ops::search [SearchQuery => SearchOutput],
    "directory.limits": publication_ops::limits [NoParams => PublicationLimits],
    "directory.set-limits": publication_ops::set_limits [PublicationLimitsInput => PublicationLimits],
    "directory.reindex": search_ops::reindex [NoParams => search_ops::ReindexOutput],
    "directory.export": backup::export [NoParams => Bundle],
    "directory.import": backup::import [ImportInput => backup::DirectoryImportOutput],
    "credential.issue": credential_ops::issue [credential_ops::CredentialIssueInput => app::SignedRecordOutput],
    "credential.list": credential_ops::list [app::MemberFilterInput => app::IssuedRecordsOutput],
    "revocation.issue": credential_ops::revoke [credential_ops::RevokeInput => app::SignedRecordOutput],
    "revocation.list": credential_ops::list_revocations [app::MemberFilterInput => app::IssuedRecordsOutput],
    "member.suspend": moderation_ops::suspend [moderation_ops::SuspendInput => app::SignedRecordOutput],
    "member.lift": moderation_ops::lift [moderation_ops::LiftInput => app::SignedRecordOutput],
    "member.decisions": moderation_ops::list [app::MemberFilterInput => app::IssuedRecordsOutput],
    "directory.standing": standing::standing_verb [StandingInput => StandingOutput],
    "directory.add-source": client_sources::add_source [client_sources::AddSourceInput => client_sources::AddSourceOutput],
    "directory.probe-info": client_sources::probe_info_verb [app::DidInput => Option<DirectoryInfoOutput>],
    "directory.remove-source": client_sources::remove_source [app::DidInput => app::RemovedOutput],
    "directory.sources": client_sources::sources [NoParams => client_sources::SourcesOutput],
    "directory.start-run": client_query::start_run [NoParams => client_query::StartRunOutput],
    "directory.query-source": client_query::query_source [client_query::QuerySourceInput => client_query::QuerySourceOutput],
    "directory.merge": client_merge::merge [client_merge::RunInput => client_merge::MergeOutput],
    "directory.run-envelope": client_merge::run_envelope [client_merge::RunEnvelopeInput => Option<client_merge::RunEnvelopeOutput>],
    "directory.check-standing": held::check_standing [held::CheckStandingInput => held::CheckStandingOutput],
    "directory.memberships": held::memberships [app::MemberFilterInput => held::MembershipsOutput],
    "directory.publish-to-source": client_sources::publish_to_source [client_sources::PublishToSourceInput => PublishOutput],
    "directory.signing-status": signing_status [NoParams => SigningStatusOutput],
    "directory.install-signing-certificate": install_signing_certificate [InstallCertificateInput => InstallCertificateOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
