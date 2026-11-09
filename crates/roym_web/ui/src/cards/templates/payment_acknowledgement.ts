/// Renders a payment-acknowledgement card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A `<time>` element's `datetime` is computed by the Hub from a number,
///   never copied from a string of the card.
/// - A time that ends a row is a `<time>` element. A time inside a sentence
///   is plain text in the string catalog message.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { labelled } from "../../dom";
import { timeElement } from "../../i18n/format";
import { t } from "../../i18n/i18n";
import { formatMinor } from "../../money";
import { methodLabel } from "../../taxonomy/taxonomy";

export interface PaymentAcknowledgementData {
  agreement?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  role?: "counterparty" | "quote-author";
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
  title.textContent = t("payment-ack-title");
  card.appendChild(title);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "payment-ack-role";
    roleP.textContent = data.role === "counterparty" ? t("payment-claimed") : t("payment-acknowledged");
    card.appendChild(roleP);
  }

  if (data?.amount_minor !== undefined && data?.currency) {
    const amountP = document.createElement("p");
    amountP.className = "payment-ack-amount";
    amountP.textContent = t("payment-amount", { amount: formatMinor(data.amount_minor, data.currency) });
    card.appendChild(amountP);
  }

  if (data?.observed_at_secs !== undefined) {
    const observedP = document.createElement("p");
    observedP.className = "payment-ack-observed";
    labelled(observedP, t("payment-observed-label"), timeElement(data.observed_at_secs));
    card.appendChild(observedP);
  }

  if (data?.method) {
    const methodP = document.createElement("p");
    methodP.className = "payment-ack-method";
    methodP.textContent = t("payment-method", { method: methodLabel(data.method) });
    card.appendChild(methodP);
  }

  if (data?.reference) {
    const refP = document.createElement("p");
    refP.className = "payment-ack-reference";
    refP.textContent = t("payment-reference", { reference: data.reference });
    card.appendChild(refP);
  }

  if (data?.supersedes) {
    const correctedP = document.createElement("p");
    correctedP.className = "payment-ack-corrected";
    correctedP.textContent = t("payment-corrected");
    card.appendChild(correctedP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "payment-ack-notice";
  noticeP.textContent = t("payment-notice");
  card.appendChild(noticeP);

  return card;
}
