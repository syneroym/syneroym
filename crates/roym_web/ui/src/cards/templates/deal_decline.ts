/// Renders a deal-decline card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { t } from "../../i18n/i18n";
import { type LocalText } from "./enquiry";

export interface DealDeclineData {
  declines?: string;
  deal?: string;
  conversation?: string;
  reason?: LocalText | string;
  /// A reason code the owner's node set. The reader's Hub shows its sentence
  /// in the reader's own language.
  code?: string;
}

/// The code's sentence, or `undefined` for a code this build does not know.
export function declineCodeSentence(code: string): string | undefined {
  switch (code) {
    case "consent-expired":
      return t("decline-code-consent-expired");
    case "offering-withdrawn":
      return t("decline-code-offering-withdrawn");
    case "no-such-slot":
      return t("decline-code-no-such-slot");
    case "slot-not-in-listing":
      return t("decline-code-slot-not-in-listing");
    default:
      return undefined;
  }
}

export function renderDealDecline(data?: DealDeclineData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-deal-decline";

  const title = document.createElement("h3");
  title.textContent = t("decline-title");
  card.appendChild(title);

  if (data?.deal) {
    const dealP = document.createElement("p");
    dealP.className = "decline-deal";
    dealP.textContent = data.deal;
    card.appendChild(dealP);
  }

  const reasonText =
    typeof data?.reason === "object" && data?.reason !== null
      ? (data.reason as { text?: string }).text
      : data?.reason;
  if (data?.code) {
    const sentence = declineCodeSentence(data.code);
    if (sentence !== undefined) {
      const codeP = document.createElement("p");
      codeP.className = "decline-code";
      codeP.textContent = sentence;
      card.appendChild(codeP);
    }
  }
  if (reasonText) {
    const noteP = document.createElement("p");
    noteP.className = "decline-reason";
    noteP.textContent = reasonText;
    card.appendChild(noteP);
  }

  return card;
}
