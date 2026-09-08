/// Renders an agreement receipt card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.
/// - Never says "verified".

import { formatMinor } from "../../money";
import { renderLink } from "../link";
import { type AgreedTermsData } from "./quote";

export interface AgreementReceiptData {
  quote_record_id?: string;
  consumer_did?: string;
  provider_did?: string;
  role?: "consumer" | "provider";
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
    where: "at-customer" | "at-provider" | "remote";
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
  title.textContent = "Agreement Receipt";
  card.appendChild(title);

  // Pair state in words:
  // "Both parties have signed these terms." / "Only the consumer has signed so far." / "Only the provider has signed so far."
  const rawPair = data?.pair_state ?? data?.pair;
  const pairState = typeof rawPair === "object" && rawPair !== null ? rawPair.state : rawPair;
  const pairRole = typeof rawPair === "object" && rawPair !== null ? rawPair.role : data?.role;

  let pairText = "Only the consumer has signed so far.";
  if (pairState === "complete") {
    pairText = "Both parties have signed these terms.";
  } else if (pairState === "half" && pairRole === "provider") {
    pairText = "Only the provider has signed so far.";
  } else if (pairRole === "provider") {
    pairText = "Only the provider has signed so far.";
  } else {
    pairText = "Only the consumer has signed so far.";
  }

  const pairP = document.createElement("p");
  pairP.className = "receipt-pair-state";
  pairP.textContent = pairText;
  card.appendChild(pairP);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "receipt-role";
    roleP.textContent = `Role: ${data.role}`;
    card.appendChild(roleP);
  }

  if (data?.consumer_did) {
    const conP = document.createElement("p");
    conP.className = "receipt-consumer";
    conP.textContent = `Consumer: ${data.consumer_did}`;
    card.appendChild(conP);
  }

  if (data?.provider_did) {
    const proP = document.createElement("p");
    proP.className = "receipt-provider";
    proP.textContent = `Provider: ${data.provider_did}`;
    card.appendChild(proP);
  }

  if (data?.quote_record_id) {
    const qidP = document.createElement("p");
    qidP.className = "receipt-quote-record";
    qidP.textContent = `Quote record: ${data.quote_record_id}`;
    card.appendChild(qidP);
  }

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

  if (t.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "receipt-scope";
    scopeP.textContent = `Scope: ${t.scope}`;
    card.appendChild(scopeP);
  }

  if (t.amount_minor !== undefined && t.currency) {
    const totalP = document.createElement("p");
    totalP.className = "receipt-total";
    totalP.textContent = `Total: ${formatMinor(t.amount_minor, t.currency)}`;
    card.appendChild(totalP);
  }

  if (t.tax_minor !== undefined && t.tax_minor !== 0 && t.currency) {
    const taxP = document.createElement("p");
    taxP.className = "receipt-tax";
    taxP.textContent = `Tax: ${formatMinor(t.tax_minor, t.currency)}`;
    card.appendChild(taxP);
  }
  if (t.fees_minor !== undefined && t.fees_minor !== 0 && t.currency) {
    const feesP = document.createElement("p");
    feesP.className = "receipt-fees";
    feesP.textContent = `Fees: ${formatMinor(t.fees_minor, t.currency)}`;
    card.appendChild(feesP);
  }

  if (t.payment_timing) {
    const timingP = document.createElement("p");
    timingP.className = "receipt-timing";
    timingP.textContent =
      t.payment_timing === "before-work"
        ? "Payment before the work"
        : "Payment after the work";
    card.appendChild(timingP);
  }

  if (t.payee) {
    const payeeP = document.createElement("p");
    payeeP.className = "receipt-payee";
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

  if (t.schedule) {
    const schedP = document.createElement("p");
    schedP.className = "receipt-schedule";
    const start = new Date(t.schedule.earliest_secs * 1000).toISOString();
    const end = new Date(t.schedule.latest_secs * 1000).toISOString();
    schedP.textContent = `Schedule: ${start} - ${end}`;
    card.appendChild(schedP);
  }

  if (t.location) {
    const locP = document.createElement("p");
    locP.className = "receipt-location";
    locP.textContent = `Location: ${t.location.where}`;
    card.appendChild(locP);

    if (t.location.address) {
      const addrP = document.createElement("p");
      addrP.className = "receipt-address";
      addrP.textContent = `Address given in this quote: ${t.location.address}`;
      card.appendChild(addrP);
    }
  }

  if (t.cancellation_terms) {
    const cancP = document.createElement("p");
    cancP.className = "receipt-cancellation";
    cancP.textContent = `Cancellation terms: ${t.cancellation_terms}`;
    card.appendChild(cancP);
  }
  if (t.refund_terms) {
    const refP = document.createElement("p");
    refP.className = "receipt-refund";
    refP.textContent = `Refund terms: ${t.refund_terms}`;
    card.appendChild(refP);
  }
  if (t.dispute_path) {
    const dispP = document.createElement("p");
    dispP.className = "receipt-dispute";
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

  if (t.quote_expires_at_secs) {
    const expDate = new Date(t.quote_expires_at_secs * 1000).toISOString();
    const expP = document.createElement("p");
    expP.className = "receipt-expiry";
    expP.textContent = `Quote expiry: ${expDate}`;
    card.appendChild(expP);
  }

  return card;
}
