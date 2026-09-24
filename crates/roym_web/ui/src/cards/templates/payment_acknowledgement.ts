/// Renders a payment-acknowledgement card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { formatMinor } from "../../money";
import { PAYMENT_ACKNOWLEDGED, PAYMENT_CLAIMED, PAYMENT_NOTICE } from "../wording";

export interface PaymentAcknowledgementData {
  agreement?: string;
  conversation?: string;
  consumer_did?: string;
  provider_did?: string;
  role?: "consumer" | "provider";
  currency?: string;
  amount_minor?: number;
  observed_at_secs?: number;
  method?: string;
  reference?: string;
  /// Set when this card's own envelope names a `supersedes` record. Not
  /// currently populated by any call site -- see the "known gaps" note in
  /// the C8 Hub UI report.
  supersedes?: string;
}

export function renderPaymentAcknowledgement(data?: PaymentAcknowledgementData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-payment-acknowledgement";

  const title = document.createElement("h3");
  title.textContent = "Payment Acknowledgement";
  card.appendChild(title);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "payment-ack-role";
    roleP.textContent = data.role === "consumer" ? PAYMENT_CLAIMED : PAYMENT_ACKNOWLEDGED;
    card.appendChild(roleP);
  }

  if (data?.amount_minor !== undefined && data?.currency) {
    const amountP = document.createElement("p");
    amountP.className = "payment-ack-amount";
    amountP.textContent = `Amount: ${formatMinor(data.amount_minor, data.currency)}`;
    card.appendChild(amountP);
  }

  if (data?.observed_at_secs !== undefined) {
    const observedP = document.createElement("p");
    observedP.className = "payment-ack-observed";
    const observed = new Date(data.observed_at_secs * 1000).toISOString();
    observedP.textContent = `Observed: ${observed}`;
    card.appendChild(observedP);
  }

  if (data?.method) {
    const methodP = document.createElement("p");
    methodP.className = "payment-ack-method";
    methodP.textContent = `Method: ${data.method}`;
    card.appendChild(methodP);
  }

  if (data?.reference) {
    const refP = document.createElement("p");
    refP.className = "payment-ack-reference";
    refP.textContent = `Reference: ${data.reference}`;
    card.appendChild(refP);
  }

  if (data?.supersedes) {
    const correctedP = document.createElement("p");
    correctedP.className = "payment-ack-corrected";
    correctedP.textContent = "This is a corrected statement.";
    card.appendChild(correctedP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "payment-ack-notice";
  noticeP.textContent = PAYMENT_NOTICE;
  card.appendChild(noticeP);

  return card;
}
