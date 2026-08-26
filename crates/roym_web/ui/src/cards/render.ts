import { CARD_TYPES } from "./registry";
import { renderUnknown } from "./unknown";
import { AgreementReceiptData, renderAgreementReceipt } from "./templates/agreement_receipt";
import { BookingProgressData, renderBookingProgress } from "./templates/booking_progress";
import { FulfilmentReceiptData, renderFulfilmentReceipt } from "./templates/fulfilment_receipt";
import { PaymentAcknowledgementData, renderPaymentAcknowledgement } from "./templates/payment_acknowledgement";
import { PaymentRequestData, renderPaymentRequest } from "./templates/payment_request";
import { QuoteData, renderQuote } from "./templates/quote";
import { RequestData, renderRequest } from "./templates/request";

export interface CardObject {
  type: string;
  version: number;
  data?: unknown;
}

export function renderCard(card: CardObject): HTMLElement {
  const isKnown = CARD_TYPES.some(([t, v]) => t === card.type && v === card.version);
  if (!isKnown) {
    return renderUnknown(card.type, card.version);
  }

  switch (card.type) {
    case "request":
      return renderRequest(card.data as RequestData | undefined);
    case "quote":
      return renderQuote(card.data as QuoteData | undefined);
    case "agreement-receipt":
      return renderAgreementReceipt(card.data as AgreementReceiptData | undefined);
    case "booking-progress":
      return renderBookingProgress(card.data as BookingProgressData | undefined);
    case "payment-request":
      return renderPaymentRequest(card.data as PaymentRequestData | undefined);
    case "payment-acknowledgement":
      return renderPaymentAcknowledgement(card.data as PaymentAcknowledgementData | undefined);
    case "fulfilment-receipt":
      return renderFulfilmentReceipt(card.data as FulfilmentReceiptData | undefined);
    default:
      return renderUnknown(card.type, card.version);
  }
}
