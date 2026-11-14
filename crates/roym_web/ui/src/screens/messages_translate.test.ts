import { afterEach, describe, expect, it, vi } from "vitest";
import { renderQuote } from "../cards/templates/quote";
import { stubRpc } from "../testing/stub-rpc";
import { fakeEngine, readerReads, unavailableEngine } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { type CardRow, renderCardActions, renderMessages } from "./messages";

const ME = "did:key:me";

const TERMS = {
  lang: "hi",
  scope: "छत की मरम्मत",
  cancellation_terms: "रद्द करने की शर्त",
  refund_terms: "वापसी की शर्त",
  dispute_path: "विवाद का रास्ता",
};
const ENGLISH = {
  lang: "en",
  scope: "Roof repair",
  cancellation_terms: "Cancellation terms",
  refund_terms: "Refund terms",
  dispute_path: "Dispute path",
};

function quoteCard(translations: unknown[] = []): CardRow {
  return {
    message_id: "m_q",
    conversation: "conv-1",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    card_type: "quote",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_q",
    data: { counterparty_did: ME, terms: TERMS, translations },
    stored_at_secs: 1,
  } as unknown as CardRow;
}

/// A quote card with its Accept button, drawn as the messages screen draws it.
function drawn(card: CardRow): HTMLElement {
  const wrap = document.createElement("div");
  wrap.appendChild(renderQuote(card.data as Record<string, unknown>));
  renderCardActions(wrap, card, "conv-1", ME, async () => {});
  return wrap;
}

function accepting(): Record<string, unknown>[] {
  const accepted: Record<string, unknown>[] = [];
  stubRpc({
    "agreement.accept": (params) => {
      accepted.push(params);
      return {};
    },
    "translation.translate": fakeEngine([]),
  });
  return accepted;
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setTargetsForTest([]);
});

describe("Accept says what the person read", () => {
  it("a quote with one version sends its own language and not machine", async () => {
    readerReads([]);
    const accepted = accepting();
    const wrap = drawn(quoteCard());
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "hi", viewed_via_mt: false });
  });

  it("a card with no text drawn is read as the original", async () => {
    const accepted = accepting();
    const wrap = document.createElement("div");
    renderCardActions(wrap, quoteCard(), "conv-1", ME, async () => {});
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "hi", viewed_via_mt: false });
  });

  it("the author's translation on screen is what is sent", async () => {
    readerReads([]);
    const accepted = accepting();
    const wrap = drawn(quoteCard([ENGLISH]));
    expect(wrap.querySelector<HTMLElement>("[data-shown-lang]")!.dataset.shownVia).toBe("author");
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "en", viewed_via_mt: false });
  });

  it("the original on screen is what is sent", async () => {
    readerReads([]);
    const accepted = accepting();
    const wrap = drawn(quoteCard([ENGLISH]));
    wrap.querySelector<HTMLButtonElement>(".record-text-switch")!.click();
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "hi", viewed_via_mt: false });
  });
});

describe("Accept after a machine translation", () => {
  async function translated(): Promise<{ wrap: HTMLElement; accepted: Record<string, unknown>[] }> {
    readerReads([]);
    setTargetsForTest(["en"]);
    const accepted = accepting();
    const wrap = drawn(quoteCard());
    wrap.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() =>
      expect(wrap.querySelector<HTMLElement>("[data-shown-lang]")!.dataset.shownVia).toBe("machine"),
    );
    return { wrap, accepted };
  }

  it("warns first, and makes no call until the person confirms", async () => {
    const { wrap, accepted } = await translated();
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    expect(wrap.querySelector(".accept-mt-warning")!.textContent).toBe(
      "You read a machine translation. The original, in Hindi, is what binds.",
    );
    expect(accepted).toEqual([]);

    wrap.querySelector<HTMLButtonElement>(".accept-mt-confirm")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "en", viewed_via_mt: true });
    expect(wrap.querySelector(".accept-mt-warning")).toBeNull();
  });

  it("cancel removes the warning and makes no call", async () => {
    const { wrap, accepted } = await translated();
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    wrap.querySelector<HTMLButtonElement>(".accept-mt-cancel")!.click();
    expect(wrap.querySelector(".accept-mt-warning")).toBeNull();
    expect(accepted).toEqual([]);
  });

  it("asking twice shows one warning", async () => {
    const { wrap } = await translated();
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    expect(wrap.querySelectorAll(".accept-mt-warning")).toHaveLength(1);
  });

  it("an engine that is unavailable leaves the original, and Accept sends it", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const accepted: Record<string, unknown>[] = [];
    stubRpc({
      "agreement.accept": (params) => {
        accepted.push(params);
        return {};
      },
      "translation.translate": unavailableEngine,
    });
    const wrap = drawn(quoteCard());
    wrap.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(wrap.querySelector(".record-text-error")).not.toBeNull());
    wrap.querySelector<HTMLButtonElement>(".accept-quote-button")!.click();
    await vi.waitFor(() => expect(accepted).toHaveLength(1));
    expect(accepted[0]).toEqual({ quote_record_id: "rec_q", viewed_lang: "hi", viewed_via_mt: false });
  });
});

describe("chat messages with a translation engine", () => {
  function conversation(messages: unknown[]) {
    stubRpc({
      whoami: () => ({ person_did: ME }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: messages.length }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages }),
      "translation.translate": fakeEngine([]),
    });
  }

  const text = (id: string, direction: string, extra: Record<string, unknown> = {}) => ({
    id,
    conversation: "conv-1",
    author: "peer-1",
    direction,
    sender_timestamp_ms: 1000,
    content_type: "text/plain",
    body_encoding: "utf8",
    body: `body ${id}`,
    languages: ["ta"],
    state: "delivered",
    ...extra,
  });

  it("only an incoming text message has a translate button", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    conversation([
      text("m_in", "incoming"),
      text("m_out", "outgoing"),
      text("m_del", "incoming", { deleted_at_secs: 5 }),
      text("m_bin", "incoming", { body_encoding: "base64", content_type: "application/octet-stream" }),
      text("m_en", "incoming", { languages: ["en"] }),
    ]);
    const host = document.createElement("div");
    await renderMessages(host);
    expect(host.querySelectorAll(".message-translate")).toHaveLength(1);
    expect(host.querySelector(".message-translate")!.closest(".message")!.textContent).toContain(
      "body m_in",
    );
  });

  it("a node with no engine draws no translate button", async () => {
    readerReads([]);
    conversation([text("m_in", "incoming")]);
    const host = document.createElement("div");
    await renderMessages(host);
    expect(host.querySelector(".message-translate")).toBeNull();
  });
});
