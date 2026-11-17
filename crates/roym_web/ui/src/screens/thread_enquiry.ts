import type { CardRow } from "./messages";

export function newestVersionMap(
  allCards: CardRow[],
  cardType: string,
  idField: string,
): Map<string, string> {
  const map = new Map<string, string>();
  for (const c of allCards) {
    if (c.card_type === cardType && c.verified && c.record_id) {
      const id = (c.data as Record<string, unknown> | undefined)?.[idField];
      if (typeof id === "string" && id) {
        const currNewestRecord = map.get(id);
        if (!currNewestRecord) {
          map.set(id, c.record_id);
        } else {
          const existing = allCards.find((x) => x.record_id === currNewestRecord);
          const cVer = c.version_count ?? 0;
          const exVer = existing?.version_count ?? 0;
          if (
            cVer > exVer ||
            (cVer === exVer && c.sender_timestamp_ms > (existing?.sender_timestamp_ms ?? 0))
          ) {
            map.set(id, c.record_id);
          }
        }
      }
    }
  }
  return map;
}

export function computeNewestRequestMap(allCards: CardRow[]): Map<string, string> {
  return newestVersionMap(allCards, "enquiry", "enquiry_id");
}

export function computeNewestQuoteMap(allCards: CardRow[]): Map<string, string> {
  return newestVersionMap(allCards, "quote", "quote_id");
}

export function isQuoteSuperseded(
  cardRow: CardRow,
  allCards: CardRow[],
  newestRequestMap: Map<string, string>,
): boolean {
  const reqRecordId = cardRow.data?.enquiry_record_id;
  if (cardRow.card_type === "quote" && cardRow.verified && reqRecordId) {
    const matchingReq = allCards.find(
      (c) =>
        c.card_type === "enquiry" &&
        (c.record_id === reqRecordId ||
          c.data?.enquiry_id === cardRow.data?.enquiry_id),
    );
    const reqId = matchingReq?.data?.enquiry_id;
    if (matchingReq && reqId) {
      const newestRecordId = newestRequestMap.get(reqId);
      if (newestRecordId && newestRecordId !== reqRecordId) {
        return true;
      }
    }
  }
  return false;
}
