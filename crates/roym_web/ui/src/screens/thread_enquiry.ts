import type { CardRow } from "./messages";

export function computeNewestRequestMap(allCards: CardRow[]): Map<string, string> {
  const newestRequestMap = new Map<string, string>();
  for (const c of allCards) {
    const isEnquiry = c.card_type === "enquiry";
    const reqId = c.data?.enquiry_id;
    if (isEnquiry && c.verified && reqId && c.record_id) {
      const currNewestRecord = newestRequestMap.get(reqId);
      if (!currNewestRecord) {
        newestRequestMap.set(reqId, c.record_id);
      } else {
        const existing = allCards.find((x) => x.record_id === currNewestRecord);
        const cVer = c.version_count ?? 0;
        const exVer = existing?.version_count ?? 0;
        if (
          cVer > exVer ||
          (cVer === exVer && c.sender_timestamp_ms > (existing?.sender_timestamp_ms ?? 0))
        ) {
          newestRequestMap.set(reqId, c.record_id);
        }
      }
    }
  }
  return newestRequestMap;
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
