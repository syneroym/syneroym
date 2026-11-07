/// Renders a booking-progress card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { codeWord, messageId, t } from "../../i18n/i18n";

export interface BookingProgressData {
  agreement?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  seq?: number;
  state?: "scheduled" | "in-progress" | "completed" | "cancelled" | "conflict" | "ended-unconfirmed";
  conflict?: "slot-taken" | "slot-unavailable";
  payment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
  fulfilment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
  track_window_ends_at_secs?: number;
  cancelled_by?: "counterparty" | "quote-author";
  cancel_reason?: string;
}

/// A `Map`, because a state comes from another node: `constructor` or
/// `__proto__` must be an unknown value, not an inherited property.
const STATE_WORDS = new Map<string, string>([
  ["scheduled", messageId("progress-state-scheduled")],
  ["in-progress", messageId("progress-state-in-progress")],
  ["completed", messageId("progress-state-completed")],
  ["cancelled", messageId("progress-state-cancelled")],
  ["conflict", messageId("progress-state-conflict")],
  ["ended-unconfirmed", messageId("progress-state-ended-unconfirmed")],
]);

const CONFLICT_SENTENCES = new Map<string, string>([
  ["slot-taken", messageId("progress-conflict-slot-taken")],
  ["slot-unavailable", messageId("progress-conflict-slot-unavailable")],
]);

function trackSentence(track: string | undefined, claimed: string, acknowledged: string): string | undefined {
  switch (track) {
    case "claimed":
      return claimed;
    case "acknowledged":
      return acknowledged;
    case "unconfirmed":
      return t("track-unconfirmed");
    default:
      return undefined;
  }
}

export function renderBookingProgress(data?: BookingProgressData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-booking-progress";

  const title = document.createElement("h3");
  title.textContent = t("progress-title");
  card.appendChild(title);

  if (data?.state) {
    const stateP = document.createElement("p");
    stateP.className = "progress-state";
    stateP.textContent = t("progress-status", { state: codeWord(STATE_WORDS, data.state) });
    card.appendChild(stateP);
  }

  const paymentSentence = trackSentence(data?.payment, t("payment-claimed"), t("payment-acknowledged"));
  if (paymentSentence) {
    const paymentP = document.createElement("p");
    paymentP.className = "progress-payment";
    paymentP.textContent = paymentSentence;
    card.appendChild(paymentP);
  }

  const fulfilmentSentence = trackSentence(data?.fulfilment, t("fulfilment-claimed"), t("fulfilment-acknowledged"));
  if (fulfilmentSentence) {
    const fulfilmentP = document.createElement("p");
    fulfilmentP.className = "progress-fulfilment";
    fulfilmentP.textContent = fulfilmentSentence;
    card.appendChild(fulfilmentP);
  }

  if (data?.conflict) {
    const conflictP = document.createElement("p");
    conflictP.className = "progress-conflict";
    conflictP.textContent = codeWord(CONFLICT_SENTENCES, data.conflict);
    card.appendChild(conflictP);
  }

  if (data?.cancel_reason) {
    const cancelP = document.createElement("p");
    cancelP.className = "progress-cancel-reason";
    cancelP.textContent = t("progress-cancel-reason", { reason: data.cancel_reason });
    card.appendChild(cancelP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "progress-notice";
  noticeP.textContent = t("progress-notice");
  card.appendChild(noticeP);

  return card;
}
