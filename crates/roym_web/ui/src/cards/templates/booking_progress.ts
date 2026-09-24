/// Renders a booking-progress card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import {
  FULFILMENT_ACKNOWLEDGED,
  FULFILMENT_CLAIMED,
  PAYMENT_ACKNOWLEDGED,
  PAYMENT_CLAIMED,
  PROGRESS_NOTICE,
  TRACK_UNCONFIRMED,
} from "../wording";

export interface BookingProgressData {
  agreement?: string;
  conversation?: string;
  consumer_did?: string;
  provider_did?: string;
  seq?: number;
  state?: "scheduled" | "in-progress" | "completed" | "cancelled" | "conflict" | "ended-unconfirmed";
  conflict?: "slot-taken" | "slot-unavailable";
  payment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
  fulfilment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
  track_window_ends_at_secs?: number;
  cancelled_by?: "consumer" | "provider";
  cancel_reason?: string;
}

const STATE_WORDS: Record<string, string> = {
  scheduled: "Scheduled",
  "in-progress": "In progress",
  completed: "Completed",
  cancelled: "Cancelled",
  conflict: "Conflict",
  "ended-unconfirmed": "Ended, unconfirmed",
};

const CONFLICT_SENTENCES: Record<string, string> = {
  "slot-taken": "Another booking took this slot first.",
  "slot-unavailable": "The provider removed this slot.",
};

function trackSentence(track: string | undefined, claimed: string, acknowledged: string): string | undefined {
  switch (track) {
    case "claimed":
      return claimed;
    case "acknowledged":
      return acknowledged;
    case "unconfirmed":
      return TRACK_UNCONFIRMED;
    default:
      return undefined;
  }
}

export function renderBookingProgress(data?: BookingProgressData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-booking-progress";

  const title = document.createElement("h3");
  title.textContent = "Booking Progress";
  card.appendChild(title);

  if (data?.state) {
    const stateP = document.createElement("p");
    stateP.className = "progress-state";
    stateP.textContent = `Status: ${STATE_WORDS[data.state] ?? data.state}`;
    card.appendChild(stateP);
  }

  const paymentSentence = trackSentence(data?.payment, PAYMENT_CLAIMED, PAYMENT_ACKNOWLEDGED);
  if (paymentSentence) {
    const paymentP = document.createElement("p");
    paymentP.className = "progress-payment";
    paymentP.textContent = paymentSentence;
    card.appendChild(paymentP);
  }

  const fulfilmentSentence = trackSentence(data?.fulfilment, FULFILMENT_CLAIMED, FULFILMENT_ACKNOWLEDGED);
  if (fulfilmentSentence) {
    const fulfilmentP = document.createElement("p");
    fulfilmentP.className = "progress-fulfilment";
    fulfilmentP.textContent = fulfilmentSentence;
    card.appendChild(fulfilmentP);
  }

  if (data?.conflict) {
    const conflictP = document.createElement("p");
    conflictP.className = "progress-conflict";
    conflictP.textContent = CONFLICT_SENTENCES[data.conflict] ?? data.conflict;
    card.appendChild(conflictP);
  }

  if (data?.cancel_reason) {
    const cancelP = document.createElement("p");
    cancelP.className = "progress-cancel-reason";
    cancelP.textContent = `Cancellation reason: ${data.cancel_reason}`;
    card.appendChild(cancelP);
  }

  const noticeP = document.createElement("p");
  noticeP.className = "progress-notice";
  noticeP.textContent = PROGRESS_NOTICE;
  card.appendChild(noticeP);

  return card;
}
