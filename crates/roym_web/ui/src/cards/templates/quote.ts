/// Renders a quote card.
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

import type { ServiceLocationKind } from "../../listings/editor";
import { serviceLocationCardLabel } from "../../listings/labels";
import { labelled } from "../../dom";
import { timeElement, formatDateTime } from "../../i18n/format";
import { t } from "../../i18n/i18n";
import { formatMinor } from "../../money";
import { renderLink } from "../link";

export interface AgreedTermsData {
  scope?: string;
  currency?: string;
  amount_minor?: number;
  tax_minor?: number;
  fees_minor?: number;
  payment_methods?: string[];
  payee?: string;
  payment_timing?: "before-work" | "after-work";
  schedule?: {
    earliest_secs: number;
    latest_secs: number;
  };
  location?: {
    where: ServiceLocationKind;
    address?: string;
    area?: unknown;
  };
  cancellation_terms?: string;
  refund_terms?: string;
  dispute_path?: string;
  quote_expires_at_secs?: number;
}

export interface QuoteData {
  quote_id?: string;
  conversation?: string;
  sequence?: number;
  request_record_id?: string;
  listing_id?: string;
  counterparty_did?: string;
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
  expired?: boolean;
}

export function renderQuote(data?: QuoteData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-quote";

  const title = document.createElement("h3");
  title.textContent = t("quote-title");
  card.appendChild(title);

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

  // 1. Scope
  if (terms.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "quote-scope";
    scopeP.textContent = t("terms-scope", { value: terms.scope });
    card.appendChild(scopeP);
  }

  // 2. Total
  if (terms.amount_minor !== undefined && terms.currency) {
    const totalP = document.createElement("p");
    totalP.className = "quote-total";
    totalP.textContent = t("terms-total", { amount: formatMinor(terms.amount_minor, terms.currency) });
    card.appendChild(totalP);
  }

  // 3. Tax and fees lines when non-zero
  if (terms.tax_minor !== undefined && terms.tax_minor !== 0 && terms.currency) {
    const taxP = document.createElement("p");
    taxP.className = "quote-tax";
    taxP.textContent = t("terms-tax", { amount: formatMinor(terms.tax_minor, terms.currency) });
    card.appendChild(taxP);
  }
  if (terms.fees_minor !== undefined && terms.fees_minor !== 0 && terms.currency) {
    const feesP = document.createElement("p");
    feesP.className = "quote-fees";
    feesP.textContent = t("terms-fees", { amount: formatMinor(terms.fees_minor, terms.currency) });
    card.appendChild(feesP);
  }

  // 4. Payment timing in words
  if (terms.payment_timing) {
    const timingP = document.createElement("p");
    timingP.className = "quote-timing";
    timingP.textContent =
      terms.payment_timing === "before-work" ? t("terms-timing-before") : t("terms-timing-after");
    card.appendChild(timingP);
  }

  // 5. Payee, labelled "Payee, as agreed in this quote"
  if (terms.payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "quote-payee";
    const isLink = terms.payee.startsWith("http://") || terms.payee.startsWith("https://");
    labelled(
      payeeP,
      t("terms-payee-label"),
      isLink ? renderLink(terms.payee) : document.createTextNode(terms.payee),
    );
    card.appendChild(payeeP);
  }

  // 6. Schedule window
  if (terms.schedule) {
    const schedP = document.createElement("p");
    schedP.className = "quote-schedule";
    labelled(
      schedP,
      t("terms-schedule-label"),
      timeElement(terms.schedule.earliest_secs),
      t("terms-range-separator"),
      timeElement(terms.schedule.latest_secs),
    );
    card.appendChild(schedP);
  }

  // 7. Location and address
  if (terms.location) {
    const locP = document.createElement("p");
    locP.className = "quote-location";
    locP.textContent = t("terms-location", { where: serviceLocationCardLabel(terms.location.where) });
    card.appendChild(locP);

    if (terms.location.address) {
      const addrP = document.createElement("p");
      addrP.className = "quote-address";
      addrP.textContent = t("terms-address", { address: terms.location.address });
      card.appendChild(addrP);
    }
  }

  // 8. Cancellation, refund and dispute text
  if (terms.cancellation_terms) {
    const cancP = document.createElement("p");
    cancP.className = "quote-cancellation";
    cancP.textContent = t("terms-cancellation", { value: terms.cancellation_terms });
    card.appendChild(cancP);
  }
  if (terms.refund_terms) {
    const refP = document.createElement("p");
    refP.className = "quote-refund";
    refP.textContent = t("terms-refund", { value: terms.refund_terms });
    card.appendChild(refP);
  }
  if (terms.dispute_path) {
    const dispP = document.createElement("p");
    dispP.className = "quote-dispute";
    const isLink = terms.dispute_path.startsWith("http://") || terms.dispute_path.startsWith("https://");
    labelled(
      dispP,
      t("terms-dispute-label"),
      isLink ? renderLink(terms.dispute_path) : document.createTextNode(terms.dispute_path),
    );
    card.appendChild(dispP);
  }

  // 9. Expiry as an absolute time plus "expired" when past
  if (terms.quote_expires_at_secs) {
    const nowSecs = Math.floor(Date.now() / 1000);
    const isPast = data?.expired || nowSecs > terms.quote_expires_at_secs;
    const expP = document.createElement("p");
    expP.className = "quote-expiry";
    if (isPast) {
      const date = formatDateTime(terms.quote_expires_at_secs);
      expP.textContent = t("quote-expires-expired", { date });
      card.appendChild(expP);

      const expiredNotice = document.createElement("p");
      expiredNotice.className = "quote-expired-notice";
      expiredNotice.textContent = t("quote-expired-notice", { date });
      card.appendChild(expiredNotice);
    } else {
      labelled(expP, t("quote-expiry-label"), timeElement(terms.quote_expires_at_secs));
      card.appendChild(expP);
    }
  }

  return card;
}
