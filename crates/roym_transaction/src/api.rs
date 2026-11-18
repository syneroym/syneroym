//! Method dispatch table for the transaction service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    fulfilment::FulfilmentReceiptPayload,
    method::{EnvelopeHistoryOutput, MethodInfo, NoParams, PingOutput, Reply, VerifyInput},
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
    transaction::{AgreementReceiptPayload, EnquiryPayload, QuotePayload, RecordVerdict},
};

use crate::app::{
    AgreementRef, HalfRecordedOutput, ListInput, agreement_ops, backup, decline_ops, enquiry_ops,
    fulfilment_ops, payment_ops, progress_ops, quote_ops, sync, thread,
};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::TRANSACTION.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "enquiry.ping": ping [NoParams => PingOutput],
    "quote.ping": ping [NoParams => PingOutput],
    "agreement.ping": ping [NoParams => PingOutput],
    "receipt.ping": ping [NoParams => PingOutput],
    "payment.ping": ping [NoParams => PingOutput],
    "fulfilment.ping": ping [NoParams => PingOutput],
    "deal.ping": ping [NoParams => PingOutput],

    "enquiry.set": enquiry_ops::set [enquiry_ops::EnquirySetInput => enquiry_ops::EnquirySetOutput],
    "enquiry.get": enquiry_ops::get [enquiry_ops::EnquiryKeyInput => Option<enquiry_ops::EnquiryView>],
    "enquiry.list": enquiry_ops::list [ListInput => enquiry_ops::EnquiryListOutput],
    "enquiry.history": enquiry_ops::history [enquiry_ops::EnquiryKeyInput => EnvelopeHistoryOutput],
    "enquiry.verify": enquiry_ops::verify [VerifyInput => RecordVerdict<EnquiryPayload>],

    "quote.set": quote_ops::set [quote_ops::QuoteSetInput => quote_ops::QuoteSetOutput],
    "quote.measure": quote_ops::measure [quote_ops::QuoteMeasureInput => quote_ops::QuoteMeasureOutput],
    "quote.get": quote_ops::get [quote_ops::QuoteKeyInput => Option<quote_ops::QuoteView>],
    "quote.list": quote_ops::list [ListInput => quote_ops::QuoteListOutput],
    "quote.history": quote_ops::history [quote_ops::QuoteKeyInput => quote_ops::QuoteHistoryOutput],
    "quote.verify": quote_ops::verify [VerifyInput => RecordVerdict<QuotePayload>],

    "deal.decline": decline_ops::decline [decline_ops::DealDeclineInput => decline_ops::DealDeclineOutput],

    "agreement.accept": agreement_ops::accept [agreement_ops::AgreementAcceptInput => agreement_ops::AgreementAcceptOutput],
    "agreement.get": agreement_ops::get [agreement_ops::QuoteRecordInput => Option<agreement_ops::AgreementView>],
    "agreement.list": agreement_ops::list [agreement_ops::AgreementListInput => agreement_ops::AgreementListOutput],
    "agreement.cancel": progress_ops::cancel [progress_ops::AgreementCancelInput => agreement_ops::AgreementView],
    "agreement.progress": progress_ops::progress [AgreementRef => progress_ops::AgreementProgressOutput],
    "agreement.verify": agreement_ops::verify [VerifyInput => RecordVerdict<AgreementReceiptPayload>],

    "payment.request": payment_ops::request [payment_ops::PaymentRequestInput => payment_ops::PaymentRequestOutput],
    "payment.acknowledge": payment_ops::acknowledge [payment_ops::PaymentAckInput => HalfRecordedOutput],
    "payment.get": payment_ops::get [payment_ops::PaymentKeyInput => payment_ops::PaymentGetOutput],
    "payment.verify": payment_ops::verify [VerifyInput => payment_ops::PaymentVerdict],

    "fulfilment.sign": fulfilment_ops::sign [fulfilment_ops::FulfilmentSignInput => HalfRecordedOutput],
    "fulfilment.get": fulfilment_ops::get [fulfilment_ops::FulfilmentKeyInput => fulfilment_ops::FulfilmentGetOutput],
    "fulfilment.verify": fulfilment_ops::verify [VerifyInput => RecordVerdict<FulfilmentReceiptPayload>],

    "transaction.sync": sync::sync [sync::SyncInput => sync::SyncOutput],
    "transaction.thread": thread::thread [thread::ThreadInput => thread::ThreadOutput],
    "transaction.export": backup::export [NoParams => Bundle],
    "transaction.import": backup::import [ImportInput => backup::TransactionImportOutput],
    "transaction.signing-status": signing_status [NoParams => SigningStatusOutput],
    "transaction.install-signing-certificate": install_signing_certificate [InstallCertificateInput => InstallCertificateOutput],
}

pub fn method_table() -> Vec<MethodInfo> {
    method_rows()
}
