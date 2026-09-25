/// Renders a payment-request card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.
/// - The payee comes from `agreement_payee` (a sibling field on the card
///   row, set by the transaction service from the signed agreement) and
///   never from this card's own signed `data` -- nothing else may claim to
///   be the payee.

import { formatMinor } from "../../money";
import { renderLink } from "../link";
import { ONE_PAYMENT_NOTICE, PAYMENT_NOTICE } from "../wording";

export interface PaymentRequestData {
  agreement?: string;
  conversation?: string;
  consumer_did?: string;
  provider_did?: string;
  currency?: string;
  amount_minor?: number;
  note?: string;
  /// Sibling fields on the card row, not part of the signed payload.
  agreement_payee?: string;
  agreement_payment_methods?: string[];
}

export function renderPaymentRequest(data?: PaymentRequestData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-payment-request";

  const title = document.createElement("h3");
  title.textContent = "Payment Request";
  card.appendChild(title);

  if (data?.amount_minor !== undefined && data?.currency) {
    const amountP = document.createElement("p");
    amountP.className = "payment-request-amount";
    amountP.textContent = `Amount: ${formatMinor(data.amount_minor, data.currency)}`;
    card.appendChild(amountP);
  }

  const oneNoticeP = document.createElement("p");
  oneNoticeP.className = "payment-request-one-notice";
  oneNoticeP.textContent = ONE_PAYMENT_NOTICE;
  card.appendChild(oneNoticeP);

  if (data?.agreement_payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "payment-request-payee";
    const label = document.createElement("span");
    label.textContent = "Payee, as agreed in this quote: ";
    payeeP.appendChild(label);
    if (data.agreement_payee.startsWith("http://") || data.agreement_payee.startsWith("https://")) {
      payeeP.appendChild(renderLink(data.agreement_payee));
    } else {
      payeeP.appendChild(document.createTextNode(data.agreement_payee));
    }
    card.appendChild(payeeP);
  }

  if (data?.note) {
    const noteP = document.createElement("p");
    noteP.className = "payment-request-note";
    noteP.textContent = `Note: ${data.note}`;
    card.appendChild(noteP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "payment-request-notice";
  noticeP.textContent = PAYMENT_NOTICE;
  card.appendChild(noticeP);

  return card;
}
