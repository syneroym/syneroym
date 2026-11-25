import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { _resetForTest, startFeed } from "../events";
import { FakeFeedSource } from "../testing/fake-event-source";
import { fakeEngine, readerReads } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { renderMessages } from "./messages.js";

const CONV = "conv-1";
const ME = "did:key:me";
/** The feed groups the events of one subject for this long before it calls a listener. */
const FEED_BURST_MS = 150;

function textMessage(id: string, body: string, direction = "incoming") {
  return {
    id,
    conversation: CONV,
    author: direction === "incoming" ? "did:key:peer" : "did:key:me",
    direction,
    sender_timestamp_ms: 1000,
    content_type: "text/plain",
    body_encoding: "utf8",
    state: "delivered",
    body,
  };
}

const REQUEST_MESSAGE = {
  ...textMessage("msg-req", ""),
  content_type: "application/vnd.roym.card+json",
  body: undefined,
};

const REQUEST_CARD = {
  message_id: "msg-req",
  conversation: CONV,
  direction: "incoming",
  sender_timestamp_ms: 1000,
  card_type: "enquiry",
  version: 1,
  known: true,
  verified: true,
  expired: false,
  issuer: "did:key:peer",
  record_id: "rec_req",
  data: { enquiry_id: "req-1", description: { lang: "en", text: "Trim my hedge" }, categories: [] },
  stored_at_secs: 1,
};

/**
 * A node that answers the calls of the messages screen from `messages` and `cards`, and
 * `extra` for any other method, or to replace an answer. An answer may be a promise, so a test
 * can hold the node back. Both arrays may grow during a test. `historyCalls` counts
 * the thread draws.
 */
function stubNode(
  messages: unknown[],
  cards: unknown[] = [],
  extra: Record<string, (params: Record<string, unknown>) => unknown> = {},
) {
  const node = { historyCalls: 0 };
  const answers: Record<string, (params: Record<string, unknown>) => unknown> = {
    whoami: () => ({ person_did: "did:key:me" }),
    "conversation.list": () => ({
      conversations: [{ id: CONV, peer_address: "peer-1", message_count: messages.length }],
    }),
    "transaction.sync": () => ({}),
    "transaction.thread": () => ({ cards }),
    "conversation.history": () => {
      node.historyCalls++;
      return { messages };
    },
    ...extra,
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      const { method, params } = JSON.parse(String(init.body)) as {
        method: string;
        params: Record<string, unknown>;
      };
      const answer = answers[method];
      const result = answer ? await answer(params) : {};
      return { ok: true, json: async () => ({ result }) };
    }),
  );
  return node;
}

/** Draw the messages screen with the event feed running, and return the screen. */
async function drawScreen(): Promise<HTMLElement> {
  startFeed();
  expect(FakeFeedSource.instance).not.toBeNull();
  const container = document.createElement("div");
  document.body.appendChild(container);
  await renderMessages(container, ME);
  return container;
}

/** The node reports a new message in the open conversation, and the feed passes it on. */
async function feedReportsChange() {
  FakeFeedSource.instance!.emit(
    "change",
    JSON.stringify({ kind: "conversation.message", subject: CONV, item: "msg-new" }),
    "cursor-new",
  );
  await vi.advanceTimersByTimeAsync(FEED_BURST_MS);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("EventSource", FakeFeedSource);
  _resetForTest();
});

afterEach(() => {
  document.body.replaceChildren();
  setTargetsForTest([]);
  _resetForTest();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Event feed integration in messages screen", () => {
  it("a_change_for_the_open_conversation_refills_the_thread", async () => {
    const messages = [textMessage("msg-1", "First message", "outgoing")];
    const node = stubNode(messages);
    const screen = await drawScreen();
    expect(node.historyCalls).toBe(1);
    expect(screen.textContent).toContain("First message");

    messages.push(textMessage("msg-2", "Second message"));
    await feedReportsChange();

    expect(node.historyCalls).toBe(2);
    expect(screen.textContent).toContain("Second message");
  });
});

describe("a feed redraw keeps open work", () => {
  it("a change for the open conversation keeps the open quote form", async () => {
    const messages: unknown[] = [REQUEST_MESSAGE];
    const node = stubNode(messages, [REQUEST_CARD]);
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".quote-request-button")!.click();
    const scope = screen.querySelector<HTMLInputElement>(".quote-scope-input")!;
    scope.value = "Trim the hedge";

    messages.push(textMessage("msg-new", "Are you free on Monday?"));
    await feedReportsChange();

    expect(screen.querySelector(".quote-scope-input")).toBe(scope);
    expect(scope.value).toBe("Trim the hedge");
    expect(node.historyCalls).toBe(1);
    expect(screen.textContent).not.toContain("Are you free on Monday?");

    screen.querySelector<HTMLButtonElement>(".cancel-quote-button")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(node.historyCalls).toBe(2);
    expect(screen.textContent).toContain("Are you free on Monday?");
  });

  it("a change keeps an open delete dialog", async () => {
    const messages: unknown[] = [textMessage("msg-1", "Hello")];
    const node = stubNode(messages);
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".delete-message")!.click();
    const dialog = screen.querySelector(".delete-dialog")!;

    messages.push(textMessage("msg-new", "Are you there?"));
    await feedReportsChange();

    expect(screen.querySelector(".delete-dialog")).toBe(dialog);
    expect(node.historyCalls).toBe(1);

    screen.querySelector<HTMLButtonElement>(".cancel-delete")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(node.historyCalls).toBe(2);
    expect(screen.textContent).toContain("Are you there?");
  });

  it("a confirmed delete draws the thread again at once", async () => {
    const message: Record<string, unknown> = textMessage("msg-1", "Hello");
    const node = stubNode([message], [], {
      "conversation.delete-message": () => {
        message.deleted_at_secs = 2;
        return {};
      },
    });
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".delete-message")!.click();

    // The dialog is still open when the delete asks for a redraw.
    screen.querySelector<HTMLButtonElement>(".confirm-delete")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(node.historyCalls).toBe(2);
    expect(screen.querySelector(".delete-dialog")).toBeNull();
    expect(screen.querySelector(".message-body.deleted")).not.toBeNull();
  });
});

/** A quote for the signed-in person, as a card message and its card row. */
const QUOTE_MESSAGE = {
  ...textMessage("msg-quote", ""),
  content_type: "application/vnd.roym.card+json",
  body: undefined,
};

const QUOTE_CARD = {
  ...REQUEST_CARD,
  message_id: "msg-quote",
  card_type: "quote",
  record_id: "rec_quote",
  data: {
    request_id: "req-1",
    counterparty_did: ME,
    terms: { lang: "hi", scope: "छत की मरम्मत" },
    translations: [],
  },
};

describe("a feed redraw that is already running", () => {
  it("keeps a quote form that the person opened while the node was answering", async () => {
    const messages: unknown[] = [REQUEST_MESSAGE];
    let release: () => void = () => {};
    let slow = false;
    stubNode(messages, [REQUEST_CARD], {
      "conversation.history": async () => {
        if (slow) await new Promise<void>((resolve) => (release = resolve));
        return { messages };
      },
    });
    const screen = await drawScreen();

    slow = true;
    messages.push(textMessage("msg-new", "Are you free on Monday?"));
    await feedReportsChange();
    screen.querySelector<HTMLButtonElement>(".quote-request-button")!.click();
    const scope = screen.querySelector<HTMLInputElement>(".quote-scope-input")!;
    scope.value = "Trim the hedge";
    release();
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.querySelector(".quote-scope-input")).toBe(scope);
    expect(scope.value).toBe("Trim the hedge");
    expect(screen.textContent).not.toContain("Are you free on Monday?");

    slow = false;
    screen.querySelector<HTMLButtonElement>(".cancel-quote-button")!.click();
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.textContent).toContain("Are you free on Monday?");
  });
});

describe("a feed redraw keeps the other open work", () => {
  it("a change keeps an open decline dialog, and a confirmed decline draws at once", async () => {
    const messages: unknown[] = [QUOTE_MESSAGE];
    const card: Record<string, unknown> = { ...QUOTE_CARD };
    const node = stubNode(messages, [card], {
      "deal.decline": () => {
        card.declined = true;
        return {};
      },
    });
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".decline-quote-button")!.click();
    const dialog = screen.querySelector(".decline-dialog")!;

    messages.push(textMessage("msg-new", "Is Monday fine?"));
    await feedReportsChange();

    expect(screen.querySelector(".decline-dialog")).toBe(dialog);
    expect(node.historyCalls).toBe(1);

    // The dialog is still open when the decline asks for a redraw.
    screen.querySelector<HTMLButtonElement>(".confirm-decline")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(node.historyCalls).toBe(2);
    expect(screen.querySelector(".decline-dialog")).toBeNull();
    expect(screen.textContent).toContain("Is Monday fine?");
  });

  it("a change keeps the warning before an acceptance", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const messages: unknown[] = [QUOTE_MESSAGE];
    const node = stubNode(messages, [QUOTE_CARD], { "translation.translate": fakeEngine([]) });
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.advanceTimersByTimeAsync(0);
    screen.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    const warning = screen.querySelector(".accept-mt-box")!;
    expect(warning).not.toBeNull();

    messages.push(textMessage("msg-new", "Is Monday fine?"));
    await feedReportsChange();

    expect(screen.querySelector(".accept-mt-box")).toBe(warning);
    expect(node.historyCalls).toBe(1);

    screen.querySelector<HTMLButtonElement>(".accept-mt-cancel")!.click();
    await vi.advanceTimersByTimeAsync(0);
    expect(node.historyCalls).toBe(2);
  });

  it("does not drop the feed listener when an event arrives during the initial load", async () => {
    startFeed();
    const container = document.createElement("div");
    document.body.appendChild(container);
    let releaseList: (() => void) | null = null;
    const listBlocked = new Promise<void>((resolve) => {
      releaseList = resolve;
    });
    let emitted = false;
    const messages: unknown[] = [textMessage("m1", "First")];
    const node = stubNode(messages, [], {
      "conversation.list": async () => {
        if (!emitted) {
          emitted = true;
          FakeFeedSource.instance!.emit(
            "change",
            JSON.stringify({ kind: "conversation.message", subject: CONV }),
            "c1",
          );
          await listBlocked;
        }
        return {
          conversations: [{ id: CONV, peer_address: "peer-1", message_count: 1 }],
        };
      },
    });

    const renderPromise = renderMessages(container, ME);
    await vi.advanceTimersByTimeAsync(FEED_BURST_MS);
    releaseList!();
    await renderPromise;

    expect(node.historyCalls).toBe(1);

    messages.push(textMessage("m2", "Second"));
    await feedReportsChange();

    expect(node.historyCalls).toBe(2);
    expect(container.textContent).toContain("Second");
  });
});

describe("what the person sends draws the thread at once", () => {
  it("a sent message shows now, and the open quote form is closed with the redraw", async () => {
    const messages: unknown[] = [REQUEST_MESSAGE];
    stubNode(messages, [REQUEST_CARD], {
      "conversation.send": () => {
        messages.push(textMessage("msg-mine", "One moment", "outgoing"));
        return {};
      },
    });
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".quote-request-button")!.click();
    expect(screen.querySelector(".quote-form")).not.toBeNull();

    screen.querySelector<HTMLInputElement>(".compose-input")!.value = "One moment";
    screen.querySelector<HTMLButtonElement>(".compose-row button")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.textContent).toContain("One moment");
    expect(screen.querySelector(".quote-form")).toBeNull();
  });

  it("a sent request shows now, and the open quote form is closed with the redraw", async () => {
    const messages: unknown[] = [REQUEST_MESSAGE];
    stubNode(messages, [REQUEST_CARD], {
      "enquiry.set": () => {
        messages.push(textMessage("msg-mine", "Please trim my lawn too", "outgoing"));
        return {};
      },
    });
    const screen = await drawScreen();
    screen.querySelector<HTMLButtonElement>(".quote-request-button")!.click();
    expect(screen.querySelector(".quote-form")).not.toBeNull();

    screen.querySelector<HTMLButtonElement>(".toggle-request-form")!.click();
    screen.querySelector<HTMLTextAreaElement>(".request-desc-input")!.value = "Please trim my lawn too";
    screen.querySelector<HTMLButtonElement>(".send-request-button")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.textContent).toContain("Please trim my lawn too");
    expect(screen.querySelector(".quote-form")).toBeNull();
  });
});
