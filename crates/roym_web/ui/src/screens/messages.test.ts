import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith, t } from "../i18n/i18n";
import {
  type CardRow,
  canRetry,
  computeNewestRequestMap,
  isQuoteSuperseded,
  messageStateWord,
  openQuoteForm,
  refusalText,
  renderMessages,
  stateHint,
} from "./messages.js";
import { _resetForTest, startFeed } from "../events";
import { stubRpc } from "../testing/stub-rpc";

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
      counterparty_did: "did:key:counterparty",
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
      counterparty_did: "did:key:counterparty",
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

class MessagesTestEventSource {
  static instance: MessagesTestEventSource | null = null;
  static readonly OPEN = 1;
  readyState = 1;
  listeners = new Map<string, Set<(e: { data: string; lastEventId: string }) => void>>();

  constructor() {
    MessagesTestEventSource.instance = this;
  }

  addEventListener(type: string, fn: (e: { data: string; lastEventId: string }) => void) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)!.add(fn);
  }

  emit(type: string, data: string, lastEventId = "") {
    for (const fn of this.listeners.get(type) || []) {
      fn({ data, lastEventId });
    }
  }

  close() {}
}

describe("Event feed integration in messages screen", () => {
  it("a_change_for_the_open_conversation_refills_the_thread", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("EventSource", MessagesTestEventSource);
    _resetForTest();

    let historyCalls = 0;
    const messages = [
      {
        id: "msg-1",
        conversation: "conv-1",
        author: "did:key:me",
        direction: "outgoing",
        sender_timestamp_ms: 1000,
        content_type: "text/plain",
        body_encoding: "utf-8",
        state: "delivered",
        stored_at_secs: 1,
        visible_seq: 1,
        verified: true,
        body: "First message",
      },
    ];

    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init: RequestInit) => {
        const body = JSON.parse(String(init.body)) as {
          method: string;
          params: Record<string, unknown>;
        };
        if (body.method === "whoami") {
          return { ok: true, json: async () => ({ result: { person_did: "did:key:me" } }) };
        }
        if (body.method === "conversation.list") {
          return {
            ok: true,
            json: async () => ({
              result: {
                conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 1 }],
              },
            }),
          };
        }
        if (body.method === "transaction.sync") {
          return { ok: true, json: async () => ({ result: {} }) };
        }
        if (body.method === "transaction.thread") {
          return { ok: true, json: async () => ({ result: { cards: [] } }) };
        }
        if (body.method === "conversation.history") {
          historyCalls++;
          return { ok: true, json: async () => ({ result: { messages } }) };
        }
        return { ok: true, json: async () => ({ result: {} }) };
      }),
    );

    startFeed();
    const es = MessagesTestEventSource.instance!;
    expect(es).toBeDefined();

    const container = document.createElement("div");
    document.body.appendChild(container);
    await renderMessages(container);

    expect(historyCalls).toBe(1);
    expect(container.textContent).toContain("First message");

    messages.push({
      id: "msg-2",
      conversation: "conv-1",
      author: "did:key:peer",
      direction: "incoming",
      sender_timestamp_ms: 2000,
      content_type: "text/plain",
      body_encoding: "utf-8",
      state: "delivered",
      stored_at_secs: 2,
      visible_seq: 2,
      verified: true,
      body: "Second message",
    });

    es.emit(
      "change",
      JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "msg-2" }),
      "c-2",
    );

    await vi.advanceTimersByTimeAsync(150);

    expect(historyCalls).toBe(2);
    expect(container.textContent).toContain("Second message");

    container.remove();
    _resetForTest();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });
});


describe("text that the person signs", () => {
  afterEach(async () => {
    await activate("en");
  });

  const MAX_BYTES = 2048;
  const SIGNED = [
    "request-data-use-notice",
    "quote-default-cancellation",
    "quote-default-refund",
    "quote-default-dispute",
  ];

  it("fits the limit the service checks, in both languages", async () => {
    for (const language of ["en", "hi"]) {
      if (language === "hi") await activateWith("hi", { hi: async () => HINDI });
      for (const id of SIGNED) {
        const bytes = new TextEncoder().encode(t(id)).length;
        expect(bytes, `${language}/${id}`).toBeGreaterThan(0);
        expect(bytes, `${language}/${id}`).toBeLessThanOrEqual(MAX_BYTES);
      }
    }
  });

  it("fills the quote terms in the language of the Hub", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" } as CardRow;
    openQuoteForm(anchor, request, "conv1", async () => {});
    const value = (cls: string) => anchor.querySelector<HTMLInputElement>(cls)?.value;
    expect(value(".quote-canc-input")).toBe(t("quote-default-cancellation"));
    expect(value(".quote-ref-input")).toBe(t("quote-default-refund"));
    expect(value(".quote-disp-input")).toBe(t("quote-default-dispute"));
    expect(value(".quote-canc-input")).toMatch(/[ऀ-ॿ]/);
  });
});

describe("the message state word", () => {
  afterEach(async () => {
    await activate("en");
  });

  it("is the state itself in English", () => {
    for (const state of ["pending", "deposited", "delivered", "failed"]) {
      expect(messageStateWord(state)).toBe(state);
    }
  });

  it("is a Hindi word in Hindi, and an unknown state is shown as it came", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    expect(messageStateWord("delivered")).toMatch(/[ऀ-ॿ]/);
    for (const state of ["constructor", "toString", "__proto__", "new-state"]) {
      expect(messageStateWord(state)).toBe(state);
    }
  });
});

describe("request form category picker reset", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("clears category chips after sending a request", async () => {
    let sentCategories: string[] | undefined;
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
      "request.set": (params) => {
        sentCategories = (params as { categories: string[] }).categories;
        return { request_id: "req-1", record_id: "rec-1" };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    // Open request form
    const toggleBtn = host.querySelector<HTMLButtonElement>("button.toggle-request-form")!;
    toggleBtn.click();

    // Fill description
    const desc = host.querySelector<HTMLTextAreaElement>(".request-desc-input")!;
    desc.value = "Need bike repair";

    // Add category in picker
    const filter = host.querySelector<HTMLInputElement>(".request-cats-input .picker-filter")!;
    filter.value = "Bicycle";
    filter.dispatchEvent(new Event("input"));
    const opt = host.querySelector<HTMLButtonElement>('.request-cats-input .picker-option[data-code="bicycle-repair"]')!;
    opt.click();

    expect(host.querySelectorAll(".request-cats-input .picker-chip")).toHaveLength(1);

    // Send request
    const sendBtn = host.querySelector<HTMLButtonElement>("button.send-request-button")!;
    sendBtn.click();

    await vi.waitFor(() => expect(sentCategories).toBeDefined());
    expect(sentCategories).toEqual(["bicycle-repair"]);
    // Category chips are gone because clear() was called
    await vi.waitFor(() =>
      expect(host.querySelectorAll(".request-cats-input .picker-chip")).toHaveLength(0),
    );
  });
});
