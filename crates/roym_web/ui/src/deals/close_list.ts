export type CloseSide =
  | "give"
  | "receive"
  | "giver"
  | "receiver"
  | "quote-author"
  | "counterparty";

export interface CloseListCommitment {
  commitment_id: string;
  giver: string;
  declared_value_minor?: number | bigint | string;
  window?: { earliest_secs: number; latest_secs: number };
  description?: { lang?: string; text?: string } | string;
  [key: string]: unknown;
}

export interface CloseListProgress {
  commitment_id: string;
  state: string;
  [key: string]: unknown;
}

export interface AgreementViewForClose {
  terms: {
    commitments: CloseListCommitment[];
    [key: string]: unknown;
  };
  commitments?: CloseListProgress[];
  role?: "quote-author" | "counterparty";
  my_role?: "quote-author" | "counterparty";
  quote_author_did?: string;
  counterparty_did?: string;
  my_did?: string;
  [key: string]: unknown;
}

export interface CloseListItem {
  commitment_id: string;
  checked: boolean;
  notStarted: boolean;
  not_started: boolean;
  value: bigint;
  declared_value_minor: bigint;
  commitment: CloseListCommitment;
}

const FINAL_STATES = new Set([
  "fulfilled",
  "acknowledged",
  "ended-unconfirmed",
  "failed",
  "cancelled",
]);

/**
 * Pure calculation of the commitments to close for one side of an agreement.
 * Skips final commitments. Commitments whose window starts after `now` are unchecked.
 */
export function closeList(
  view: AgreementViewForClose,
  side: CloseSide,
  now: number,
): CloseListItem[] {
  const myRole =
    view.role ||
    view.my_role ||
    (view.my_did && view.quote_author_did
      ? view.my_did === view.quote_author_did
        ? "quote-author"
        : "counterparty"
      : "quote-author");

  const isGivingSide = side === "give" || side === "giver";
  const isReceivingSide = side === "receive" || side === "receiver";

  const progressMap = new Map<string, string>();
  if (view.commitments) {
    for (const cp of view.commitments) {
      progressMap.set(cp.commitment_id, cp.state);
    }
  }

  const result: CloseListItem[] = [];

  for (const comm of view.terms?.commitments || []) {
    // Check if commitment belongs to requested side
    if (isGivingSide) {
      if (comm.giver !== myRole) continue;
    } else if (isReceivingSide) {
      if (comm.giver === myRole) continue;
    } else {
      if (comm.giver !== side) continue;
    }

    // Skip final commitments
    const state = progressMap.get(comm.commitment_id);
    if (state && FINAL_STATES.has(state)) {
      continue;
    }

    const notStarted = Boolean(comm.window && comm.window.earliest_secs > now);
    const checked = !notStarted;
    const value = BigInt(comm.declared_value_minor ?? 0);

    result.push({
      commitment_id: comm.commitment_id,
      checked,
      notStarted,
      not_started: notStarted,
      value,
      declared_value_minor: value,
      commitment: comm,
    });
  }

  return result;
}
