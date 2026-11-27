import { t } from "../i18n/i18n";
import { formatMinor } from "../money";

export type DealTotalsRole = "quote-author" | "counterparty";

export interface DealTotalsCommitment {
  giver: string;
  declared_value_minor?: number | bigint | string;
  [key: string]: unknown;
}

export type Commitment = DealTotalsCommitment;

export interface DealTotalsResult {
  give: bigint;
  receive: bigint;
  difference: bigint;
}

/**
 * Pure calculation of deal totals and difference.
 * difference = receive - give
 */
export function dealTotals(
  commitments: DealTotalsCommitment[],
  me: DealTotalsRole,
): DealTotalsResult {
  let give = 0n;
  let receive = 0n;
  for (const c of commitments) {
    const val = BigInt(c.declared_value_minor ?? 0);
    if (c.giver === me) {
      give += val;
    } else {
      receive += val;
    }
  }
  return { give, receive, difference: receive - give };
}

export function updateDealTotalsElements(
  elements: { giveEl: HTMLElement; receiveEl: HTMLElement; diffEl: HTMLElement },
  totals: DealTotalsResult,
  currency: string,
): void {
  elements.giveEl.textContent = t("deal-you-give", { amount: formatMinor(totals.give, currency) });
  elements.receiveEl.textContent = t("deal-you-receive", { amount: formatMinor(totals.receive, currency) });

  if (totals.difference > 0n) {
    elements.diffEl.style.display = "block";
    elements.diffEl.textContent = t("deal-diff-you-receive-more", { amount: formatMinor(totals.difference, currency) });
  } else if (totals.difference < 0n) {
    elements.diffEl.style.display = "block";
    elements.diffEl.textContent = t("deal-diff-you-give-more", { amount: formatMinor(-totals.difference, currency) });
  } else {
    elements.diffEl.style.display = "none";
    elements.diffEl.textContent = "";
  }
}

export function renderDealTotalsBlock(totals: DealTotalsResult, currency: string): HTMLElement {
  const totalsBlock = document.createElement("div");
  totalsBlock.className = "quote-totals-block";
  const giveP = document.createElement("p");
  giveP.className = "quote-total-give";
  const receiveP = document.createElement("p");
  receiveP.className = "quote-total-receive";
  totalsBlock.append(giveP, receiveP);

  const diffP = document.createElement("p");
  diffP.className = "quote-total-diff";
  updateDealTotalsElements({ giveEl: giveP, receiveEl: receiveP, diffEl: diffP }, totals, currency);
  if (totals.difference !== 0n) {
    totalsBlock.appendChild(diffP);
  }
  return totalsBlock;
}
