/// Renders a commitment-failure card: the giver of one commitment says that
/// its reservation was lost, so that part of the deal cannot go ahead.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess. A reason this build does not know shows as it came.

import { codeWord, messageId, t } from "../../i18n/i18n";
import { recordValue } from "../../records/record_text";

export interface CommitmentFailureData {
  agreement?: string;
  conversation?: string;
  commitment_id?: string;
  reason?: string;
}

const REASON_SENTENCES = new Map<string, string>([
  ["reservation-lost", messageId("commitment-failure-reservation-lost")],
]);

export function renderCommitmentFailure(data?: CommitmentFailureData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-commitment-failure";

  const title = document.createElement("h3");
  title.textContent = t("commitment-failure-title");
  card.appendChild(title);

  if (data?.commitment_id) {
    const commitmentP = document.createElement("p");
    commitmentP.className = "failure-commitment";
    commitmentP.append(recordValue("", data.commitment_id));
    card.appendChild(commitmentP);
  }

  if (data?.reason) {
    const reasonP = document.createElement("p");
    reasonP.className = "failure-reason";
    reasonP.textContent = codeWord(REASON_SENTENCES, data.reason);
    card.appendChild(reasonP);
  }

  return card;
}
