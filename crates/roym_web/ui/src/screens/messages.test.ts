import { describe, expect, it } from "vitest";
import {
  type CardRow,
  canRetry,
  computeNewestRequestMap,
  isQuoteSuperseded,
  refusalText,
  stateHint,
} from "./messages.js";

describe("newestRequestMap and quote superseded notice", () => {
  const reqV1: CardRow = {
    message_id: "m_req1",
    conversation: "conv1",
    direction: "outgoing",
    sender_timestamp_ms: 1000,
    card_type: "request",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_req_v1",
    data: {
      request_id: "req_canonical_1",
      sequence: 1,
    },
    version_count: 1,
    stored_at_secs: 1,
  };

  const reqV2: CardRow = {
    message_id: "m_req2",
    conversation: "conv1",
    direction: "outgoing",
    sender_timestamp_ms: 2000,
    card_type: "request",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_req_v2",
    data: {
      request_id: "req_canonical_1", // same sequence and canonical request_id
      sequence: 1,
    },
    version_count: 2,
    stored_at_secs: 2,
  };

  const quoteAgainstV1: CardRow = {
    message_id: "m_quo1",
    conversation: "conv1",
    direction: "incoming",
    sender_timestamp_ms: 1500,
    card_type: "quote",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_quo_1",
    data: {
      quote_id: "quo_1",
      request_record_id: "rec_req_v1",
      request_id: "req_canonical_1",
      consumer_did: "did:key:consumer",
    },
    stored_at_secs: 2,
  };

  const quoteAgainstV2: CardRow = {
    message_id: "m_quo2",
    conversation: "conv1",
    direction: "incoming",
    sender_timestamp_ms: 2500,
    card_type: "quote",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_quo_2",
    data: {
      quote_id: "quo_2",
      request_record_id: "rec_req_v2",
      request_id: "req_canonical_1",
      consumer_did: "did:key:consumer",
    },
    stored_at_secs: 3,
  };

  it("correctly identifies the newest request version by version_count", () => {
    const allCards = [reqV1, reqV2, quoteAgainstV1, quoteAgainstV2];
    const map = computeNewestRequestMap(allCards);
    expect(map.get("req_canonical_1")).toBe("rec_req_v2");
  });

  it("flags quote answering v1 as superseded and quote answering v2 as NOT superseded", () => {
    const allCards = [reqV1, reqV2, quoteAgainstV1, quoteAgainstV2];
    const map = computeNewestRequestMap(allCards);

    expect(isQuoteSuperseded(quoteAgainstV1, allCards, map)).toBe(true);
    expect(isQuoteSuperseded(quoteAgainstV2, allCards, map)).toBe(false);
  });
});

describe("refusalText", () => {
  it("explains a rate limit in plain words", () => {
    expect(refusalText("rate-limited")).toBe(
      "Not accepted: this person limits messages from new contacts. Try again later.",
    );
  });

  it("shows any other reason as given", () => {
    expect(refusalText("too-large")).toBe("Not accepted: too-large.");
  });
});

describe("a deposited message", () => {
  it("shows a hint that the other side is offline", () => {
    expect(stateHint("deposited")).toBe(
      "The other side is offline. The message waits in their mailbox.",
    );
  });

  it("has no hint for the other states", () => {
    for (const state of ["pending", "delivered", "failed"] as const) {
      expect(stateHint(state)).toBeNull();
    }
  });

  it("has no retry button, and only a failed message has one", () => {
    expect(canRetry({ state: "deposited" })).toBe(false);
    expect(canRetry({ state: "pending" })).toBe(false);
    expect(canRetry({ state: "delivered" })).toBe(false);
    expect(canRetry({ state: "failed" })).toBe(true);
    expect(canRetry({ state: "failed", restored: true })).toBe(false);
    expect(canRetry({ state: "failed", deleted_at_secs: 1 })).toBe(false);
  });
});
