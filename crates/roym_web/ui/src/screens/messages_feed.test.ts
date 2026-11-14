import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { _resetForTest, startFeed } from "../events";
import { FakeFeedSource } from "../testing/fake-event-source";
import { renderMessages } from "./messages.js";

const CONV = "conv-1";
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
  card_type: "request",
  version: 1,
  known: true,
  verified: true,
  expired: false,
  issuer: "did:key:peer",
  record_id: "rec_req",
  data: { request_id: "req-1", lang: "en", description: "Trim my hedge", categories: [] },
  stored_at_secs: 1,
};

/**
 * A node that answers the calls of the messages screen from `messages` and `cards`, and
 * `extra` for any other method. Both arrays may grow during a test. `historyCalls` counts
 * the thread draws.
 */
function stubNode(
  messages: unknown[],
  cards: unknown[] = [],
  extra: Record<string, () => unknown> = {},
) {
  const node = { historyCalls: 0 };
  const answers: Record<string, () => unknown> = {
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
      const { method } = JSON.parse(String(init.body)) as { method: string };
      const answer = answers[method];
      return { ok: true, json: async () => ({ result: answer ? answer() : {} }) };
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
  await renderMessages(container);
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
  _resetForTest();
  vi.useRealTimers();
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
