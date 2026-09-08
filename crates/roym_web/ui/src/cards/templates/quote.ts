/// Renders a quote card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

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
    where: "at-customer" | "at-provider" | "remote";
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
  consumer_did?: string;
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
    where: "at-customer" | "at-provider" | "remote";
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
  title.textContent = "Quote";
  card.appendChild(title);

  const t: AgreedTermsData = {
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
  if (t.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "quote-scope";
    scopeP.textContent = `Scope: ${t.scope}`;
    card.appendChild(scopeP);
  }

  // 2. Total
  if (t.amount_minor !== undefined && t.currency) {
    const totalP = document.createElement("p");
    totalP.className = "quote-total";
    totalP.textContent = `Total: ${formatMinor(t.amount_minor, t.currency)}`;
    card.appendChild(totalP);
  }

  // 3. Tax and fees lines when non-zero
  if (t.tax_minor !== undefined && t.tax_minor !== 0 && t.currency) {
    const taxP = document.createElement("p");
    taxP.className = "quote-tax";
    taxP.textContent = `Tax: ${formatMinor(t.tax_minor, t.currency)}`;
    card.appendChild(taxP);
  }
  if (t.fees_minor !== undefined && t.fees_minor !== 0 && t.currency) {
    const feesP = document.createElement("p");
    feesP.className = "quote-fees";
    feesP.textContent = `Fees: ${formatMinor(t.fees_minor, t.currency)}`;
    card.appendChild(feesP);
  }

  // 4. Payment timing in words
  if (t.payment_timing) {
    const timingP = document.createElement("p");
    timingP.className = "quote-timing";
    timingP.textContent =
      t.payment_timing === "before-work"
        ? "Payment before the work"
        : "Payment after the work";
    card.appendChild(timingP);
  }

  // 5. Payee, labelled "Payee, as agreed in this quote"
  if (t.payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "quote-payee";
    const label = document.createElement("span");
    label.textContent = "Payee, as agreed in this quote: ";
    payeeP.appendChild(label);
    if (t.payee.startsWith("http://") || t.payee.startsWith("https://")) {
      payeeP.appendChild(renderLink(t.payee));
    } else {
      payeeP.appendChild(document.createTextNode(t.payee));
    }
    card.appendChild(payeeP);
  }

  // 6. Schedule window
  if (t.schedule) {
    const schedP = document.createElement("p");
    schedP.className = "quote-schedule";
    const start = new Date(t.schedule.earliest_secs * 1000).toISOString();
    const end = new Date(t.schedule.latest_secs * 1000).toISOString();
    schedP.textContent = `Schedule: ${start} - ${end}`;
    card.appendChild(schedP);
  }

  // 7. Location and address
  if (t.location) {
    const locP = document.createElement("p");
    locP.className = "quote-location";
    locP.textContent = `Location: ${t.location.where}`;
    card.appendChild(locP);

    if (t.location.address) {
      const addrP = document.createElement("p");
      addrP.className = "quote-address";
      addrP.textContent = `Address given in this quote: ${t.location.address}`;
      card.appendChild(addrP);
    }
  }

  // 8. Cancellation, refund and dispute text
  if (t.cancellation_terms) {
    const cancP = document.createElement("p");
    cancP.className = "quote-cancellation";
    cancP.textContent = `Cancellation terms: ${t.cancellation_terms}`;
    card.appendChild(cancP);
  }
  if (t.refund_terms) {
    const refP = document.createElement("p");
    refP.className = "quote-refund";
    refP.textContent = `Refund terms: ${t.refund_terms}`;
    card.appendChild(refP);
  }
  if (t.dispute_path) {
    const dispP = document.createElement("p");
    dispP.className = "quote-dispute";
    const label = document.createElement("span");
    label.textContent = "Dispute path: ";
    dispP.appendChild(label);
    if (t.dispute_path.startsWith("http://") || t.dispute_path.startsWith("https://")) {
      dispP.appendChild(renderLink(t.dispute_path));
    } else {
      dispP.appendChild(document.createTextNode(t.dispute_path));
    }
    card.appendChild(dispP);
  }

  // 9. Expiry as an absolute time plus "expired" when past
  if (t.quote_expires_at_secs) {
    const nowSecs = Math.floor(Date.now() / 1000);
    const isPast = data?.expired || nowSecs > t.quote_expires_at_secs;
    const expDate = new Date(t.quote_expires_at_secs * 1000).toISOString();
    const expP = document.createElement("p");
    expP.className = "quote-expiry";
    if (isPast) {
      expP.textContent = `Expires: ${expDate} (expired)`;
    } else {
      expP.textContent = `Expires: ${expDate}`;
    }
    card.appendChild(expP);

    if (isPast) {
      const expiredNotice = document.createElement("p");
      expiredNotice.className = "quote-expired-notice";
      expiredNotice.textContent = `This quote expired on ${expDate}. Ask for a new one.`;
      card.appendChild(expiredNotice);
    }
  }

  return card;
}
