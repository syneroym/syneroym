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
import { languageName } from "../../i18n/text_lang";
import { formatMinor } from "../../money";
import {
  recordText,
  recordValue,
  textFields,
  type TextVersion,
} from "../../records/record_text";
import { commitmentDetailsView } from "../details";
import { renderLink } from "../link";
import { dealTotals, type Commitment as TotalCommitment } from "../../deals/deal_totals";
import { renderVersionSummary, type VersionSummary } from "../../deals/version_summary";

export interface QuoteTranslationData {
  lang: string;
  scope: string;
  cancellation_terms: string;
  refund_terms: string;
  dispute_path: string;
}

export interface AgreedTermsData {
  lang?: string;
  scope?: string | { lang?: string; text?: string };
  currency?: string;
  amount_minor?: number;
  commitments?: Array<Record<string, unknown>>;
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
  cancellation_terms?: string | { lang?: string; text?: string };
  refund_terms?: string | { lang?: string; text?: string };
  dispute_path?: string | { lang?: string; text?: string };
  quote_expires_at_secs?: number;
}

export interface QuoteData {
  quote_id?: string;
  conversation?: string;
  sequence?: number;
  request_record_id?: string;
  listing_id?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  counterparty_name?: string;
  quote_author_name?: string;
  peer_name?: string;
  role?: "counterparty" | "quote-author";
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
  version_number?: number;
  summary?: VersionSummary;
}

export function renderQuote(data?: QuoteData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-quote";

  if (data?.summary) {
    card.appendChild(renderVersionSummary(data.summary));
  }

  const title = document.createElement("h3");
  title.textContent = t("quote-title");
  card.appendChild(title);

  const versionNumber = data?.version_number ?? data?.sequence ?? 1;
  const verP = document.createElement("p");
  verP.className = "quote-version";
  verP.textContent = t("deal-version", { n: String(versionNumber) });
  card.appendChild(verP);

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

  const scopeText =
    typeof terms.scope === "object" && terms.scope !== null
      ? (terms.scope as { text?: string }).text
      : terms.scope;
  const cancText =
    typeof terms.cancellation_terms === "object" && terms.cancellation_terms !== null
      ? (terms.cancellation_terms as { text?: string }).text
      : terms.cancellation_terms;
  const refText =
    typeof terms.refund_terms === "object" && terms.refund_terms !== null
      ? (terms.refund_terms as { text?: string }).text
      : terms.refund_terms;
  const dispText =
    typeof terms.dispute_path === "object" && terms.dispute_path !== null
      ? (terms.dispute_path as { text?: string }).text
      : terms.dispute_path;
  const termsLang =
    (typeof terms.scope === "object" && terms.scope !== null
      ? (terms.scope as { lang?: string }).lang
      : undefined) ??
    terms.lang ??
    "";

  let totalMinor = terms.amount_minor;
  let payee = terms.payee;
  if (terms.commitments && Array.isArray(terms.commitments)) {
    const currencyCommitments = terms.commitments.filter(
      (c) => c.kind === "currency" || c.resource_kind === "currency",
    );
    if (totalMinor === undefined && currencyCommitments.length > 0) {
      totalMinor = currencyCommitments.reduce(
        (sum, c) => sum + (typeof c.declared_value_minor === "number" ? c.declared_value_minor : 0),
        0,
      );
    }
    if (!payee) {
      const withPayee = currencyCommitments.find((c) => typeof c.payee === "string");
      if (withPayee && typeof withPayee.payee === "string") payee = withPayee.payee;
    }
  }

  interface QuoteTextFields {
    scope?: string;
    cancellation_terms?: string;
    refund_terms?: string;
    dispute_path?: string;
  }

  const versions: TextVersion<QuoteTextFields>[] = [
    {
      lang: termsLang,
      fields: {
        scope: scopeText,
        cancellation_terms: cancText,
        refund_terms: refText,
        dispute_path: dispText,
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

    // 0. Commitments
    if (terms.commitments && Array.isArray(terms.commitments)) {
      for (const c of terms.commitments) {
        const comP = document.createElement("p");
        comP.className = `quote-commitment quote-commitment-${c.commitment_id ?? ""}`;
        const rawDesc =
          typeof c.description === "object" && c.description !== null
            ? (c.description as { text?: string }).text
            : String(c.description ?? "");
        let desc = rawDesc;
        if (c.kind === "currency" && typeof c.declared_value_minor === "number" && terms.currency) {
          desc = formatMinor(c.declared_value_minor, terms.currency);
        } else if (!desc && c.kind === "currency") {
          desc = t("quote-payment-description");
        }
        const safeDesc = desc ?? "";
        const isAuthorGiver = c.giver === "quote-author";
        const isViewer =
          (isAuthorGiver && data?.role === "quote-author") ||
          (!isAuthorGiver && (data?.role === "counterparty" || !data?.role));
        const authorName =
          data?.quote_author_name ?? data?.peer_name ?? t("receipt-role-author");
        const otherName = isAuthorGiver
          ? authorName
          : (data?.counterparty_name ?? t("receipt-role-recipient"));
        const lineText = isViewer
          ? t("commitment-you-give", { description: safeDesc })
          : t("commitment-other-gives", { name: otherName, description: safeDesc });
        comP.textContent = lineText;
        nodes.push(comP);

        if (typeof c.declared_value_minor === "number" && terms.currency && c.kind !== "currency") {
          const valP = document.createElement("p");
          valP.className = `quote-declared-value quote-declared-value-${c.commitment_id ?? ""}`;
          valP.textContent = t("commitment-declared-value", {
            amount: formatMinor(c.declared_value_minor, terms.currency),
          });
          nodes.push(valP);
        }

        // `details` has no author translations, so it is drawn with the
        // original language of the quote on every version.
        const detailsView = commitmentDetailsView(c, terms.currency, termsLang);
        if (detailsView) nodes.push(detailsView);
      }
    }

    // 1. Scope
    if (v.fields.scope) {
      const scopeP = document.createElement("p");
      scopeP.className = "quote-scope";
      labelled(scopeP, t("terms-scope-label"), recordValue(v.lang, v.fields.scope));
      nodes.push(scopeP);
    }

    // 2. Totals
    if (terms.commitments && Array.isArray(terms.commitments) && terms.commitments.length > 0) {
      const totalsBlock = document.createElement("div");
      totalsBlock.className = "quote-totals-block";
      const role = data?.role === "quote-author" ? "quote-author" : "counterparty";
      const totals = dealTotals(terms.commitments as unknown as TotalCommitment[], role);
      const curr = terms.currency ?? "INR";

      const giveP = document.createElement("p");
      giveP.className = "quote-total-give";
      giveP.textContent = t("deal-you-give", { amount: formatMinor(totals.give, curr) });

      const receiveP = document.createElement("p");
      receiveP.className = "quote-total-receive";
      receiveP.textContent = t("deal-you-receive", { amount: formatMinor(totals.receive, curr) });

      totalsBlock.append(giveP, receiveP);

      if (totals.difference > 0n) {
        const diffP = document.createElement("p");
        diffP.className = "quote-total-diff";
        diffP.textContent = t("deal-diff-you-receive-more", { amount: formatMinor(totals.difference, curr) });
        totalsBlock.append(diffP);
      } else if (totals.difference < 0n) {
        const diffP = document.createElement("p");
        diffP.className = "quote-total-diff";
        diffP.textContent = t("deal-diff-you-give-more", { amount: formatMinor(-totals.difference, curr) });
        totalsBlock.append(diffP);
      }
      nodes.push(totalsBlock);
    } else if (totalMinor !== undefined && terms.currency) {
      const totalP = document.createElement("p");
      totalP.className = "quote-total";
      totalP.textContent = t("terms-total", { amount: formatMinor(totalMinor, terms.currency) });
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
    if (payee) {
      const payeeP = document.createElement("p");
      payeeP.className = "quote-payee";
      const isLink = payee.startsWith("http://") || payee.startsWith("https://");
      labelled(
        payeeP,
        t("terms-payee-label"),
        isLink ? renderLink(payee) : document.createTextNode(payee),
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
        labelled(addrP, t("terms-address-label"), recordValue(terms.lang ?? "", terms.location.address));
        nodes.push(addrP);
      }
    }

    // 8. Cancellation, refund and dispute text
    if (v.fields.cancellation_terms) {
      const cancP = document.createElement("p");
      cancP.className = "quote-cancellation";
      labelled(cancP, t("terms-cancellation-label"), recordValue(v.lang, v.fields.cancellation_terms));
      nodes.push(cancP);
    }
    if (v.fields.refund_terms) {
      const refP = document.createElement("p");
      refP.className = "quote-refund";
      labelled(refP, t("terms-refund-label"), recordValue(v.lang, v.fields.refund_terms));
      nodes.push(refP);
    }
    if (v.fields.dispute_path) {
      const dispP = document.createElement("p");
      dispP.className = "quote-dispute";
      const isLink = v.fields.dispute_path.startsWith("http://") || v.fields.dispute_path.startsWith("https://");
      const val = isLink ? renderLink(v.fields.dispute_path) : v.fields.dispute_path;
      labelled(dispP, t("terms-dispute-label"), recordValue(v.lang, val));
      nodes.push(dispP);
    }

    // The original binds, whoever made the version on screen
    if (v.byAuthor || v.machine) {
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
  }, textFields<QuoteTextFields>(["scope", "cancellation_terms", "refund_terms", "dispute_path"]),
  data?.quote_id !== undefined ? `quote:${data.quote_id}` : undefined);

  return card;
}
