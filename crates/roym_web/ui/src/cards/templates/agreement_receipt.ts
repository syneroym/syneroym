/// Renders an agreement receipt card.
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
/// - Never says "verified".

import type { ServiceLocationKind } from "../../listings/editor";
import { serviceLocationCardLabel } from "../../listings/labels";
import { labelled } from "../../dom";
import { timeElement } from "../../i18n/format";
import { t } from "../../i18n/i18n";
import { formatMinor } from "../../money";
import { recordValue } from "../../records/record_text";
import { renderLink } from "../link";
import { type AgreedTermsData } from "./quote";

export interface AgreementReceiptData {
  quote_record_id?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  role?: "counterparty" | "quote-author";
  pair_state?: "none" | "half" | "complete" | { state: string; role?: string };
  pair?: "none" | "half" | "complete" | { state: string; role?: string };
  terms?: AgreedTermsData;
  scope?: string;
  currency?: string;
  amount_minor?: number;
  tax_minor?: number;
  fees_minor?: number;
  payee?: string;
  payment_timing?: "before-work" | "after-work";
  schedule?: {
    earliest_secs: number;
    latest_secs: number;
  };
  location?: {
    where: ServiceLocationKind;
    address?: string;
  };
  cancellation_terms?: string;
  refund_terms?: string;
  dispute_path?: string;
  quote_expires_at_secs?: number;
  agreement_id?: string;
}

export function renderAgreementReceipt(data?: AgreementReceiptData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-agreement-receipt";

  const title = document.createElement("h3");
  title.textContent = t("receipt-title");
  card.appendChild(title);

  // Pair state in words:
  // "Both parties have signed these terms." / "Only the person the quote was sent to has signed so far." / "Only the person who wrote the quote has signed so far."
  const rawPair = data?.pair_state ?? data?.pair;
  const pairState = typeof rawPair === "object" && rawPair !== null ? rawPair.state : rawPair;
  const pairRole = typeof rawPair === "object" && rawPair !== null ? rawPair.role : data?.role;

  let pairText = t("receipt-pair-half-recipient");
  if (pairState === "complete") {
    pairText = t("receipt-pair-complete");
  } else if (pairState === "half" && pairRole === "quote-author") {
    pairText = t("receipt-pair-half-author");
  } else if (pairRole === "quote-author") {
    pairText = t("receipt-pair-half-author");
  } else {
    pairText = t("receipt-pair-half-recipient");
  }

  const pairP = document.createElement("p");
  pairP.className = "receipt-pair-state";
  pairP.textContent = pairText;
  card.appendChild(pairP);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "receipt-role";
    roleP.textContent =
      data.role === "quote-author" ? t("receipt-role-author") : t("receipt-role-recipient");
    card.appendChild(roleP);
  }

  if (data?.counterparty_did) {
    const conP = document.createElement("p");
    conP.className = "receipt-counterparty";
    conP.textContent = t("receipt-quote-to", { did: data.counterparty_did });
    card.appendChild(conP);
  }

  if (data?.quote_author_did) {
    const proP = document.createElement("p");
    proP.className = "receipt-author";
    proP.textContent = t("receipt-quote-by", { did: data.quote_author_did });
    card.appendChild(proP);
  }

  if (data?.quote_record_id) {
    const qidP = document.createElement("p");
    qidP.className = "receipt-quote-record";
    qidP.textContent = t("receipt-quote-record", { id: data.quote_record_id });
    card.appendChild(qidP);
  }

  const terms: AgreedTermsData = {
    ...data?.terms,
    ...(data?.scope !== undefined ? { scope: data.scope } : {}),
    ...(data?.currency !== undefined ? { currency: data.currency } : {}),
    ...(data?.amount_minor !== undefined ? { amount_minor: data.amount_minor } : {}),
    ...(data?.tax_minor !== undefined ? { tax_minor: data.tax_minor } : {}),
    ...(data?.fees_minor !== undefined ? { fees_minor: data.fees_minor } : {}),
    ...(data?.payee !== undefined ? { payee: data.payee } : {}),
    ...(data?.payment_timing !== undefined ? { payment_timing: data.payment_timing } : {}),
    ...(data?.schedule !== undefined ? { schedule: data.schedule } : {}),
    ...(data?.location !== undefined ? { location: data.location } : {}),
    ...(data?.cancellation_terms !== undefined ? { cancellation_terms: data.cancellation_terms } : {}),
    ...(data?.refund_terms !== undefined ? { refund_terms: data.refund_terms } : {}),
    ...(data?.dispute_path !== undefined ? { dispute_path: data.dispute_path } : {}),
    ...(data?.quote_expires_at_secs !== undefined ? { quote_expires_at_secs: data.quote_expires_at_secs } : {}),
  };

  if (terms.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "receipt-scope";
    labelled(scopeP, t("terms-scope-label"), recordValue(terms.lang ?? "", terms.scope));
    card.appendChild(scopeP);
  }

  if (terms.amount_minor !== undefined && terms.currency) {
    const totalP = document.createElement("p");
    totalP.className = "receipt-total";
    totalP.textContent = t("terms-total", { amount: formatMinor(terms.amount_minor, terms.currency) });
    card.appendChild(totalP);
  }

  if (terms.tax_minor !== undefined && terms.tax_minor !== 0 && terms.currency) {
    const taxP = document.createElement("p");
    taxP.className = "receipt-tax";
    taxP.textContent = t("terms-tax", { amount: formatMinor(terms.tax_minor, terms.currency) });
    card.appendChild(taxP);
  }
  if (terms.fees_minor !== undefined && terms.fees_minor !== 0 && terms.currency) {
    const feesP = document.createElement("p");
    feesP.className = "receipt-fees";
    feesP.textContent = t("terms-fees", { amount: formatMinor(terms.fees_minor, terms.currency) });
    card.appendChild(feesP);
  }

  if (terms.payment_timing) {
    const timingP = document.createElement("p");
    timingP.className = "receipt-timing";
    timingP.textContent =
      terms.payment_timing === "before-work" ? t("terms-timing-before") : t("terms-timing-after");
    card.appendChild(timingP);
  }

  if (terms.payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "receipt-payee";
    const isLink = terms.payee.startsWith("http://") || terms.payee.startsWith("https://");
    labelled(
      payeeP,
      t("terms-payee-label"),
      isLink ? renderLink(terms.payee) : document.createTextNode(terms.payee),
    );
    card.appendChild(payeeP);
  }

  if (terms.schedule) {
    const schedP = document.createElement("p");
    schedP.className = "receipt-schedule";
    labelled(
      schedP,
      t("terms-schedule-label"),
      timeElement(terms.schedule.earliest_secs),
      t("terms-range-separator"),
      timeElement(terms.schedule.latest_secs),
    );
    card.appendChild(schedP);
  }

  if (terms.location) {
    const locP = document.createElement("p");
    locP.className = "receipt-location";
    locP.textContent = t("terms-location", { where: serviceLocationCardLabel(terms.location.where) });
    card.appendChild(locP);

    if (terms.location.address) {
      const addrP = document.createElement("p");
      addrP.className = "receipt-address";
      labelled(addrP, t("terms-address-label"), recordValue(terms.lang ?? "", terms.location.address));
      card.appendChild(addrP);
    }
  }

  if (terms.cancellation_terms) {
    const cancP = document.createElement("p");
    cancP.className = "receipt-cancellation";
    labelled(cancP, t("terms-cancellation-label"), recordValue(terms.lang ?? "", terms.cancellation_terms));
    card.appendChild(cancP);
  }
  if (terms.refund_terms) {
    const refP = document.createElement("p");
    refP.className = "receipt-refund";
    labelled(refP, t("terms-refund-label"), recordValue(terms.lang ?? "", terms.refund_terms));
    card.appendChild(refP);
  }
  if (terms.dispute_path) {
    const dispP = document.createElement("p");
    dispP.className = "receipt-dispute";
    const isLink = terms.dispute_path.startsWith("http://") || terms.dispute_path.startsWith("https://");
    const val = isLink ? renderLink(terms.dispute_path) : terms.dispute_path;
    labelled(dispP, t("terms-dispute-label"), recordValue(terms.lang ?? "", val));
    card.appendChild(dispP);
  }

  if (terms.quote_expires_at_secs) {
    const expP = document.createElement("p");
    expP.className = "receipt-expiry";
    labelled(expP, t("receipt-expiry-label"), timeElement(terms.quote_expires_at_secs));
    card.appendChild(expP);
  }

  return card;
}
