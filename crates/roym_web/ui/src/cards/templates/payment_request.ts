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

import { labelled } from "../../dom";
import { t } from "../../i18n/i18n";
import { formatMinor } from "../../money";
import { renderLink } from "../link";

export interface PaymentRequestData {
  agreement?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
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
  title.textContent = t("payment-request-title");
  card.appendChild(title);

  if (data?.amount_minor !== undefined && data?.currency) {
    const amountP = document.createElement("p");
    amountP.className = "payment-request-amount";
    amountP.textContent = t("payment-amount", { amount: formatMinor(data.amount_minor, data.currency) });
    card.appendChild(amountP);
  }

  const oneNoticeP = document.createElement("p");
  oneNoticeP.className = "payment-request-one-notice";
  oneNoticeP.textContent = t("one-payment-notice");
  card.appendChild(oneNoticeP);

  if (data?.agreement_payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "payment-request-payee";
    const isLink =
      data.agreement_payee.startsWith("http://") || data.agreement_payee.startsWith("https://");
    labelled(
      payeeP,
      t("terms-payee-label"),
      isLink ? renderLink(data.agreement_payee) : document.createTextNode(data.agreement_payee),
    );
    card.appendChild(payeeP);
  }

  if (data?.note) {
    const noteP = document.createElement("p");
    noteP.className = "payment-request-note";
    noteP.textContent = t("payment-note", { note: data.note });
    card.appendChild(noteP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "payment-request-notice";
  noticeP.textContent = t("payment-notice");
  card.appendChild(noticeP);

  return card;
}
