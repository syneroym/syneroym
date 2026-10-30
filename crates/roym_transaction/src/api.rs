//! Method dispatch table for the transaction service.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput},
    booking::BookingProgressPayload,
    fulfilment::FulfilmentReceiptPayload,
    method::{EnvelopeHistoryOutput, MethodInfo, NoParams, PingOutput, Reply, VerifyInput},
    services,
    signing::verbs::{
        InstallCertificateInput, InstallCertificateOutput, SigningStatusOutput,
        install_signing_certificate, signing_status,
    },
    transaction::{AgreementReceiptPayload, QuotePayload, RecordVerdict, RequestPayload},
};

use crate::app::{
    AgreementRef, ListInput, agreement_ops, backup, booking_ops, fulfilment_ops, payment_ops,
    quote_ops, request_ops, sync, thread,
};

async fn ping<H: AppHost>(_host: &H, _: NoParams) -> Reply<PingOutput> {
    Ok(PingOutput { service: services::TRANSACTION.name.to_string() })
}

syneroym_roym_core::service_methods! {
    "request.ping": ping [NoParams => PingOutput],
    "quote.ping": ping [NoParams => PingOutput],
    "agreement.ping": ping [NoParams => PingOutput],
    "receipt.ping": ping [NoParams => PingOutput],
    "booking.ping": ping [NoParams => PingOutput],
    "payment.ping": ping [NoParams => PingOutput],
    "fulfilment.ping": ping [NoParams => PingOutput],

    "request.set": request_ops::set [request_ops::RequestSetInput => request_ops::RequestSetOutput],
    "request.get": request_ops::get [request_ops::RequestKeyInput => Option<request_ops::RequestView>],
    "request.list": request_ops::list [ListInput => request_ops::RequestListOutput],
    "request.history": request_ops::history [request_ops::RequestKeyInput => EnvelopeHistoryOutput],
    "request.verify": request_ops::verify [VerifyInput => RecordVerdict<RequestPayload>],

    "quote.set": quote_ops::set [quote_ops::QuoteSetInput => quote_ops::QuoteSetOutput],
    "quote.get": quote_ops::get [quote_ops::QuoteKeyInput => Option<quote_ops::QuoteView>],
    "quote.list": quote_ops::list [ListInput => quote_ops::QuoteListOutput],
    "quote.history": quote_ops::history [quote_ops::QuoteKeyInput => EnvelopeHistoryOutput],
    "quote.verify": quote_ops::verify [VerifyInput => RecordVerdict<QuotePayload>],
    "quote.decline": quote_ops::decline [quote_ops::QuoteDeclineInput => quote_ops::QuoteDeclineOutput],

    "agreement.accept": agreement_ops::accept [agreement_ops::QuoteRecordInput => agreement_ops::AgreementAcceptOutput],
    "agreement.get": agreement_ops::get [agreement_ops::QuoteRecordInput => Option<agreement_ops::AgreementView>],
    "agreement.list": agreement_ops::list [agreement_ops::AgreementListInput => agreement_ops::AgreementListOutput],
    "agreement.verify": agreement_ops::verify [VerifyInput => RecordVerdict<AgreementReceiptPayload>],

    "booking.get": booking_ops::booking_get [AgreementRef => booking_ops::BookingView],
    "booking.list": booking_ops::booking_list [booking_ops::BookingListInput => booking_ops::BookingListOutput],
    "booking.start": booking_ops::booking_start [AgreementRef => booking_ops::BookingView],
    "booking.cancel": booking_ops::booking_cancel [booking_ops::BookingCancelInput => booking_ops::BookingView],
    "booking.history": booking_ops::history [AgreementRef => booking_ops::BookingHistoryOutput],
    "booking.verify": booking_ops::verify [VerifyInput => RecordVerdict<BookingProgressPayload>],

    "payment.request": payment_ops::request [payment_ops::PaymentRequestInput => payment_ops::PaymentRequestOutput],
    "payment.acknowledge": payment_ops::acknowledge [payment_ops::PaymentAckInput => payment_ops::HalfRecordedOutput],
    "payment.get": payment_ops::get [AgreementRef => payment_ops::PaymentGetOutput],
    "payment.verify": payment_ops::verify [VerifyInput => payment_ops::PaymentVerdict],

    "fulfilment.sign": fulfilment_ops::sign [AgreementRef => payment_ops::HalfRecordedOutput],
    "fulfilment.get": fulfilment_ops::get [AgreementRef => fulfilment_ops::FulfilmentGetOutput],
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
