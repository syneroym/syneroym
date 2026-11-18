import { CARD_TYPES } from "./registry";
import { renderUnknown } from "./unknown";
import { AgreementReceiptData, renderAgreementReceipt } from "./templates/agreement_receipt";
import { AgreementProgressData, renderAgreementProgress } from "./templates/agreement_progress";
import { EnquiryData, renderEnquiry } from "./templates/enquiry";
import { FulfilmentReceiptData, renderFulfilmentReceipt } from "./templates/fulfilment_receipt";
import { DealDeclineData, renderDealDecline } from "./templates/deal_decline";
import { PaymentAcknowledgementData, renderPaymentAcknowledgement } from "./templates/payment_acknowledgement";
import { PaymentRequestData, renderPaymentRequest } from "./templates/payment_request";
import { QuoteData, renderQuote } from "./templates/quote";

export interface CardObject {
  type: string;
  version: number;
  data?: unknown;
  /// Sibling fields on the thread's card row (never inside the signed
  /// `data`), threaded through only for the payment-request template.
  agreement_payee?: string;
  agreement_payment_methods?: string[];
}

export function renderCard(card: CardObject): HTMLElement {
  const isKnown = CARD_TYPES.some(([t, v]) => t === card.type && v === card.version);
  if (!isKnown) {
    return renderUnknown(card.type, card.version);
  }

  switch (card.type) {
    case "enquiry":
      return renderEnquiry(card.data as EnquiryData | undefined);
    case "quote":
      return renderQuote(card.data as QuoteData | undefined);
    case "agreement-receipt":
      return renderAgreementReceipt(card.data as AgreementReceiptData | undefined);
    case "agreement-progress":
    case "booking-progress":
      return renderAgreementProgress(card.data as AgreementProgressData | undefined);
    case "payment-request":
      return renderPaymentRequest({
        ...(card.data as PaymentRequestData | undefined),
        agreement_payee: card.agreement_payee,
        agreement_payment_methods: card.agreement_payment_methods,
      });
    case "payment-acknowledgement":
      return renderPaymentAcknowledgement(card.data as PaymentAcknowledgementData | undefined);
    case "fulfilment-receipt":
      return renderFulfilmentReceipt(card.data as FulfilmentReceiptData | undefined);
    case "deal-decline":
      return renderDealDecline(card.data as DealDeclineData | undefined);
    default:
      return renderUnknown(card.type, card.version);
  }
}
