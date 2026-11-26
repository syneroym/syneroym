export type DealTotalsRole = "quote-author" | "counterparty";

export interface DealTotalsCommitment {
  giver: string;
  declared_value_minor?: number | bigint | string;
  [key: string]: unknown;
}

/**
 * Pure calculation of deal totals and difference.
 * difference = receive - give
 */
export function dealTotals(
  commitments: DealTotalsCommitment[],
  me: DealTotalsRole,
): { give: bigint; receive: bigint; difference: bigint } {
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
