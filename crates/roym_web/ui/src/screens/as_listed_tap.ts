// The "Agree as listed" panel on a directory hit.
//
// A person sees every term before they tap: the price, who is paid, the
// cancellation, refund and dispute texts, the payment timing, how long the
// owner's quote stays valid, and how long their own consent holds. The panel
// never sends a tap when a term is missing; it says why instead.

import { type MergedHit, runEnvelope } from "../directory/search";
import { text } from "../dom";
import { manifestFor, type ExtensionRef } from "../extensions/registry";
import { currentLanguage, t } from "../i18n/i18n";
import { formatMinor } from "../money";
import { call } from "../rpc";

interface LocalText {
  lang: string;
  text: string;
  translations?: Array<{ lang: string; text: string }>;
}

interface ListingPayload {
  lang: string;
  translations?: Array<{ lang: string; title?: string; summary?: string }>;
  extension?: ExtensionRef | null;
  acceptance?: "instant" | "confirm-first";
  payment?: {
    currency: string;
    model: string;
    amount_minor?: number;
    payee: string;
  } | null;
  deal_terms?: {
    cancellation_terms: LocalText;
    refund_terms: LocalText;
    dispute_path: LocalText;
    quote_valid_secs: number;
    payment_timing?: "before-work" | "after-work" | null;
  } | null;
}

interface VerifyAnswer {
  verified: boolean;
  payload?: ListingPayload;
  reason?: string;
}

/// Renders the panel under a verified hit. `openThread` shows the conversation
/// after the tap is sent.
export function renderAsListedTap(
  host: HTMLElement,
  hit: MergedHit,
  runId: string,
  openThread: (conversationId: string) => Promise<void>,
): void {
  const panel = document.createElement("div");
  panel.className = "as-listed-tap";
  panel.appendChild(text("p", t("as-listed-loading"), "as-listed-loading"));
  host.appendChild(panel);

  void (async () => {
    let envelope: string | null;
    try {
      envelope = await runEnvelope(runId, hit.record_id);
    } catch {
      panel.textContent = t("directory-engage-failed", { detail: "" });
      return;
    }
    if (!envelope) {
      panel.textContent = t("directory-engage-gone");
      return;
    }
    let verdict: VerifyAnswer;
    try {
      verdict = await call("listing.verify", { envelope });
    } catch {
      panel.textContent = t("as-listed-bad-offering");
      return;
    }
    const payload = verdict.payload;
    if (!verdict.verified || !payload) {
      panel.textContent = t("as-listed-bad-offering");
      return;
    }
    const extension = payload.extension ?? undefined;
    if (extension && manifestFor(extension)?.hooks?.includes("reserve")) {
      panel.textContent = t("as-listed-slot-offering");
      return;
    }
    const payment = payload.payment;
    const terms = payload.deal_terms;
    if (!payment || payment.model !== "fixed" || payment.amount_minor === undefined || !terms) {
      panel.textContent = t("as-listed-not-complete");
      return;
    }

    const viewedLang = currentLanguage();
    const viaMt =
      payload.lang !== viewedLang &&
      !(payload.translations ?? []).some((tr) => tr.lang === viewedLang);
    const holdSecs = payload.acceptance === "instant" ? 3_600 : 48 * 3_600;
    const validHours = String(Math.round(terms.quote_valid_secs / 3600));
    const tapHours = String(Math.round(holdSecs / 3600));

    panel.textContent = "";
    panel.appendChild(
      text("p", t("as-listed-price", { amount: formatMinor(payment.amount_minor, payment.currency) }), "as-listed-price"),
    );
    panel.appendChild(
      text("p", t("as-listed-will-be-paid-to", { payee: payment.payee }), "as-listed-payee"),
    );
    panel.appendChild(text("p", t("as-listed-cancellation", { text: terms.cancellation_terms.text }), "as-listed-cancellation"));
    panel.appendChild(text("p", t("as-listed-refund", { text: terms.refund_terms.text }), "as-listed-refund"));
    panel.appendChild(text("p", t("as-listed-dispute", { text: terms.dispute_path.text }), "as-listed-dispute"));
    if (terms.payment_timing) {
      const timing =
        terms.payment_timing === "before-work"
          ? t("listing-payment-timing-before")
          : t("listing-payment-timing-after");
      panel.appendChild(text("p", t("as-listed-payment-timing", { timing }), "as-listed-timing"));
    }
    panel.appendChild(
      text("p", t("as-listed-quote-valid", { hours: validHours }), "as-listed-quote-valid"),
    );
    panel.appendChild(text("p", t("as-listed-tap-holds", { hours: tapHours }), "as-listed-tap-holds"));

    if (viaMt) {
      panel.appendChild(text("p", t("as-listed-mt-warning"), "as-listed-mt-warning"));
    }

    const sendBtn = text("button", t("as-listed-send-button"), "button as-listed-send") as HTMLButtonElement;
    const err = text("p", "", "as-listed-send-error");
    sendBtn.onclick = async () => {
      sendBtn.disabled = true;
      err.textContent = "";
      try {
        const conv = await call<{ conversation_id: string }>("conversation.open", {
          address: hit.conversation_address,
        });
        await call("enquiry.set", {
          conversation: conv.conversation_id,
          as_listed: {
            envelope,
            valid_for_secs: holdSecs,
            viewed_lang: viewedLang,
            viewed_via_mt: viaMt,
          },
          data_use_notice: t("request-data-use-notice"),
        });
        err.textContent = t("as-listed-sent-waiting");
        await openThread(conv.conversation_id);
      } catch (e) {
        sendBtn.disabled = false;
        err.textContent = t("as-listed-send-failed", { detail: String(e) });
      }
    };
    panel.append(sendBtn, err);
  })();
}
