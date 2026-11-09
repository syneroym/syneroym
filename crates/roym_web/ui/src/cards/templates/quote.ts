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
import { langAttr, languageName } from "../../i18n/text_lang";
import { formatMinor } from "../../money";
import { recordText, type TextVersion } from "../../records/record_text";
import { renderLink } from "../link";

export interface QuoteTranslationData {
  lang: string;
  scope: string;
  cancellation_terms: string;
  refund_terms: string;
  dispute_path: string;
}

export interface AgreedTermsData {
  lang?: string;
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
  translations?: QuoteTranslationData[];
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

  interface QuoteTextFields {
    scope?: string;
    cancellation_terms?: string;
    refund_terms?: string;
    dispute_path?: string;
  }

  const versions: TextVersion<QuoteTextFields>[] = [
    {
      lang: terms.lang ?? "",
      fields: {
        scope: terms.scope,
        cancellation_terms: terms.cancellation_terms,
        refund_terms: terms.refund_terms,
        dispute_path: terms.dispute_path,
      },
      byAuthor: false,
    },
    ...(data?.translations ?? []).map((tr) => ({
      lang: tr.lang,
      fields: {
        scope: tr.scope,
        cancellation_terms: tr.cancellation_terms,
        refund_terms: tr.refund_terms,
        dispute_path: tr.dispute_path,
      },
      byAuthor: true,
    })),
  ];

  recordText(card, versions, (v) => {
    const nodes: Node[] = [];

    // 1. Scope
    if (v.fields.scope) {
      const scopeP = document.createElement("p");
      scopeP.className = "quote-scope";
      const valSpan = document.createElement("span");
      valSpan.textContent = v.fields.scope;
      const attr = langAttr(v.lang);
      if (attr) valSpan.setAttribute("lang", attr);
      labelled(scopeP, t("terms-scope-label"), valSpan);
      nodes.push(scopeP);
    }

    // 2. Total
    if (terms.amount_minor !== undefined && terms.currency) {
      const totalP = document.createElement("p");
      totalP.className = "quote-total";
      totalP.textContent = t("terms-total", { amount: formatMinor(terms.amount_minor, terms.currency) });
      nodes.push(totalP);
    }

    // 3. Tax and fees lines when non-zero
    if (terms.tax_minor !== undefined && terms.tax_minor !== 0 && terms.currency) {
      const taxP = document.createElement("p");
      taxP.className = "quote-tax";
      taxP.textContent = t("terms-tax", { amount: formatMinor(terms.tax_minor, terms.currency) });
      nodes.push(taxP);
    }
    if (terms.fees_minor !== undefined && terms.fees_minor !== 0 && terms.currency) {
      const feesP = document.createElement("p");
      feesP.className = "quote-fees";
      feesP.textContent = t("terms-fees", { amount: formatMinor(terms.fees_minor, terms.currency) });
      nodes.push(feesP);
    }

    // 4. Payment timing in words
    if (terms.payment_timing) {
      const timingP = document.createElement("p");
      timingP.className = "quote-timing";
      timingP.textContent =
        terms.payment_timing === "before-work" ? t("terms-timing-before") : t("terms-timing-after");
      nodes.push(timingP);
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
      nodes.push(payeeP);
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
      nodes.push(schedP);
    }

    // 7. Location and address
    if (terms.location) {
      const locP = document.createElement("p");
      locP.className = "quote-location";
      locP.textContent = t("terms-location", { where: serviceLocationCardLabel(terms.location.where) });
      nodes.push(locP);

      if (terms.location.address) {
        const addrP = document.createElement("p");
        addrP.className = "quote-address";
        const valSpan = document.createElement("span");
        valSpan.textContent = terms.location.address;
        const attr = langAttr(terms.lang ?? "");
        if (attr) valSpan.setAttribute("lang", attr);
        labelled(addrP, t("terms-address-label"), valSpan);
        nodes.push(addrP);
      }
    }

    // 8. Cancellation, refund and dispute text
    if (v.fields.cancellation_terms) {
      const cancP = document.createElement("p");
      cancP.className = "quote-cancellation";
      const valSpan = document.createElement("span");
      valSpan.textContent = v.fields.cancellation_terms;
      const attr = langAttr(v.lang);
      if (attr) valSpan.setAttribute("lang", attr);
      labelled(cancP, t("terms-cancellation-label"), valSpan);
      nodes.push(cancP);
    }
    if (v.fields.refund_terms) {
      const refP = document.createElement("p");
      refP.className = "quote-refund";
      const valSpan = document.createElement("span");
      valSpan.textContent = v.fields.refund_terms;
      const attr = langAttr(v.lang);
      if (attr) valSpan.setAttribute("lang", attr);
      labelled(refP, t("terms-refund-label"), valSpan);
      nodes.push(refP);
    }
    if (v.fields.dispute_path) {
      const dispP = document.createElement("p");
      dispP.className = "quote-dispute";
      const isLink = v.fields.dispute_path.startsWith("http://") || v.fields.dispute_path.startsWith("https://");
      const valSpan = document.createElement("span");
      const attr = langAttr(v.lang);
      if (attr) valSpan.setAttribute("lang", attr);
      if (isLink) {
        valSpan.appendChild(renderLink(v.fields.dispute_path));
      } else {
        valSpan.textContent = v.fields.dispute_path;
      }
      labelled(dispP, t("terms-dispute-label"), valSpan);
      nodes.push(dispP);
    }

    // Warning line when author translation is shown (P1)
    if (v.byAuthor) {
      const bindsP = document.createElement("p");
      bindsP.className = "quote-original-binds";
      bindsP.textContent = t("record-text-original-binds", {
        language: languageName(versions[0].lang),
      });
      nodes.push(bindsP);
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
        nodes.push(expP);

        const expiredNotice = document.createElement("p");
        expiredNotice.className = "quote-expired-notice";
        expiredNotice.textContent = t("quote-expired-notice", { date });
        nodes.push(expiredNotice);
      } else {
        labelled(expP, t("quote-expiry-label"), timeElement(terms.quote_expires_at_secs));
        nodes.push(expP);
      }
    }

    return nodes;
  });

  return card;
}
