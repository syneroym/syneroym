//! Method dispatch table for the conversation service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    method::{
        MethodInfo, NoParams, PingOutput, Reply,
        conversation::{ChangesInput, ChangesOutput, SendInput, SendOutput},
    },
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
};

use crate::app::{backup, group, messages};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::CONVERSATION.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "conversation.ping": ping [NoParams => PingOutput],
    "conversation.open": messages::open [messages::OpenInput => messages::OpenOutput],
    "conversation.list": messages::list [messages::ConversationListInput => messages::ConversationListOutput],
    "conversation.send": messages::send [SendInput => SendOutput],
    "conversation.history": messages::history [messages::HistoryInput => messages::HistoryOutput],
    "conversation.changes": messages::changes [ChangesInput => ChangesOutput],
    "conversation.delivery-status": messages::delivery_status [messages::MessageIdInput => messages::DeliveryStatusOutput],
    "conversation.outbox": messages::outbox [NoParams => messages::OutboxOutput],
    "conversation.retry": messages::retry [messages::MessageIdInput => messages::RetryOutput],
    "conversation.delete-message": messages::delete_message [messages::DeleteMessageInput => messages::DeleteMessageOutput],
    "conversation.search": messages::search [messages::SearchInput => messages::SearchOutput],
    "conversation.transcript-digest": messages::transcript_digest [messages::ConversationRef => messages::TranscriptDigestOutput],
    "conversation.export": backup::export [NoParams => Bundle],
    "conversation.import": backup::import [ImportInput => backup::ConversationImportOutput],
    "group.create": group::create [group::GroupCreateInput => group::GroupCreateOutput],
    "group.rename": group::rename [group::GroupRenameInput => group::GroupRenameOutput],
    "group.add-member": group::add_member [group::GroupMemberInput => group::GroupAddOutput],
    "group.remove-member": group::remove_member [group::GroupMemberInput => group::GroupRemoveOutput],
    "group.info": group::info [messages::ConversationRef => group::GroupInfoOutput],
    "group.sync": group::sync [messages::ConversationRef => group::GroupSyncOutput],
    "group.hide": group::hide [messages::ConversationRef => group::AdmissionOutput],
    "group.unhide": group::unhide [messages::ConversationRef => group::GroupUnhideOutput],
    "conversation.signing-status": signing_status [NoParams => SigningStatusOutput],
    "conversation.install-signing-certificate": install_signing_certificate [InstallCertificateInput => InstallCertificateOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
