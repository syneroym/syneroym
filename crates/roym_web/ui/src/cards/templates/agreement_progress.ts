/// Renders an agreement-progress card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { codeWord, messageId, t } from "../../i18n/i18n";

export interface CommitmentProgressData {
  commitment_id: string;
  state:
    | "promised"
    | "payment-requested"
    | "claimed"
    | "fulfilled"
    | "acknowledged"
    | "ended-unconfirmed"
    | "failed"
    | "cancelled";
  /// Why the commitment ended `failed` or `cancelled`.
  reason?: string;
  giver_claim?: boolean;
  track_end_secs?: number;
}

export interface AgreementProgressData {
  agreement?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  seq?: number;
  state?: "agreed" | "closed" | "cancelled" | "not-formed" | "scheduled" | "in-progress" | "completed" | "conflict" | "ended-unconfirmed";
  state_reason?: string;
  cancel_reason?: string;
  commitments?: CommitmentProgressData[];
  conflict?: "slot-taken" | "slot-unavailable";
  payment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
  fulfilment?: "none" | "claimed" | "acknowledged" | "unconfirmed";
}

const STATE_WORDS = new Map<string, string>([
  ["agreed", messageId("progress-state-agreed")],
  ["closed", messageId("progress-state-closed")],
  ["cancelled", messageId("progress-state-cancelled")],
  ["not-formed", messageId("progress-state-not-formed")],
  ["scheduled", messageId("progress-state-scheduled")],
  ["in-progress", messageId("progress-state-in-progress")],
  ["completed", messageId("progress-state-completed")],
  ["conflict", messageId("progress-state-conflict")],
  ["ended-unconfirmed", messageId("progress-state-ended-unconfirmed")],
]);

const COMMITMENT_STATE_WORDS = new Map<string, string>([
  ["promised", messageId("commitment-state-promised")],
  ["payment-requested", messageId("commitment-state-payment-requested")],
  ["claimed", messageId("commitment-state-claimed")],
  ["fulfilled", messageId("commitment-state-fulfilled")],
  ["acknowledged", messageId("commitment-state-acknowledged")],
  ["ended-unconfirmed", messageId("commitment-state-ended-unconfirmed")],
  ["failed", messageId("commitment-state-failed")],
  ["cancelled", messageId("commitment-state-cancelled")],
]);

const COMMITMENT_REASON_WORDS = new Map<string, string>([
  ["reservation-lost", messageId("commitment-reason-reservation-lost")],
  ["other-commitment-failed", messageId("commitment-reason-other-commitment-failed")],
]);

const CONFLICT_SENTENCES = new Map<string, string>([
  ["slot-taken", messageId("progress-conflict-slot-taken")],
  ["slot-unavailable", messageId("progress-conflict-slot-unavailable")],
  ["quote-superseded", messageId("quote-superseded")],
  ["quote-expired", messageId("quote-expired")],
  ["progress-fork", messageId("progress-fork")],
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

export function renderAgreementProgress(data?: AgreementProgressData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-agreement-progress card-booking-progress";

  const title = document.createElement("h3");
  title.textContent = t("progress-title");
  card.appendChild(title);

  if (data?.state) {
    const stateP = document.createElement("p");
    stateP.className = "progress-state";
    stateP.textContent = t("progress-status", { state: codeWord(STATE_WORDS, data.state) });
    card.appendChild(stateP);
  }

  if (data?.commitments && Array.isArray(data.commitments)) {
    for (const c of data.commitments) {
      const comP = document.createElement("p");
      comP.className = `progress-commitment progress-commitment-${c.commitment_id}`;
      const stateWord = codeWord(COMMITMENT_STATE_WORDS, c.state);
      const reasonWord = c.reason ? ` (${codeWord(COMMITMENT_REASON_WORDS, c.reason)})` : "";
      comP.textContent = `${c.commitment_id}: ${stateWord}${reasonWord}`;
      card.appendChild(comP);
    }
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

  const conflictCode = data?.conflict ?? data?.state_reason;
  if (conflictCode) {
    const conflictP = document.createElement("p");
    conflictP.className = "progress-conflict";
    conflictP.textContent = codeWord(CONFLICT_SENTENCES, conflictCode);
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

export const renderBookingProgress = renderAgreementProgress;
export type BookingProgressData = AgreementProgressData;
