import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith, t } from "../i18n/i18n";
import {
  type CardRow,
  canRetry,
  computeNewestQuoteMap,
  computeNewestRequestMap,
  isQuoteSuperseded,
  messageElement,
  messageStateWord,
  openQuoteForm,
  readQuoteForm,
  refusalText,
  renderCardActions,
  renderMessages,
  stateHint,
} from "./messages.js";
import type { WidgetView } from "../widgets/view";
import { stubRpc } from "../testing/stub-rpc";

describe("newestRequestMap and quote superseded notice", () => {
  const reqV1: CardRow = {
    message_id: "m_req1",
    conversation: "conv1",
    direction: "outgoing",
    sender_timestamp_ms: 1000,
    card_type: "enquiry",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_req_v1",
    data: {
      enquiry_id: "req_canonical_1",
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
    card_type: "enquiry",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_req_v2",
    data: {
      enquiry_id: "req_canonical_1", // same sequence and canonical enquiry_id
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
      enquiry_record_id: "rec_req_v1",
      enquiry_id: "req_canonical_1",
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
      enquiry_record_id: "rec_req_v2",
      enquiry_id: "req_canonical_1",
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

  it("identifies newest quote version and shows accept/decline only on the newest", () => {
    const quoteV1: CardRow = {
      message_id: "m_q1",
      conversation: "conv1",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      card_type: "quote",
      version: 1,
      known: true,
      verified: true,
      expired: false,
      record_id: "rec_q1",
      version_count: 1,
      data: {
        quote_id: "quo_1",
        counterparty_did: "did:key:me",
      },
      stored_at_secs: 1,
    };
    const quoteV2: CardRow = {
      message_id: "m_q2",
      conversation: "conv1",
      direction: "incoming",
      sender_timestamp_ms: 2000,
      card_type: "quote",
      version: 1,
      known: true,
      verified: true,
      expired: false,
      record_id: "rec_q2",
      version_count: 2,
      data: {
        quote_id: "quo_1",
        counterparty_did: "did:key:me",
      },
      stored_at_secs: 2,
    };

    const allQuotes = [quoteV1, quoteV2];
    const qMap = computeNewestQuoteMap(allQuotes);
    expect(qMap.get("quo_1")).toBe("rec_q2");

    const host1 = document.createElement("div");
    renderCardActions(host1, quoteV1, "conv1", "did:key:me", async () => {}, qMap);
    expect(host1.querySelector(".accept-quote-button")).toBeNull();
    expect(host1.querySelector(".decline-quote-button")).toBeNull();

    const host2 = document.createElement("div");
    renderCardActions(host2, quoteV2, "conv1", "did:key:me", async () => {}, qMap);
    expect(host2.querySelector(".accept-quote-button")).not.toBeNull();
    expect(host2.querySelector(".decline-quote-button")).not.toBeNull();
  });

  it("hides accept/decline when deal_state is agreed, declined, expired, or not-formed", () => {
    const states = ["agreed", "declined", "expired", "not-formed", "closed", "cancelled"];
    for (const st of states) {
      const q: CardRow = {
        message_id: "m_q",
        conversation: "conv1",
        direction: "incoming",
        sender_timestamp_ms: 1000,
        card_type: "quote",
        version: 1,
        known: true,
        verified: true,
        expired: false,
        record_id: "rec_q",
        deal_state: st,
        data: {
          quote_id: "quo_1",
          counterparty_did: "did:key:me",
        },
        stored_at_secs: 1,
      };
      const host = document.createElement("div");
      renderCardActions(host, q, "conv1", "did:key:me", async () => {});
      expect(host.querySelector(".accept-quote-button")).toBeNull();
      expect(host.querySelector(".decline-quote-button")).toBeNull();
    }
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

  it("quote form sends lang from its control in terms, starting at UI language", async () => {
    let sentTerms: Record<string, unknown> | undefined;
    stubRpc({
      "quote.set": (params) => {
        sentTerms = (params as { terms: Record<string, unknown> }).terms;
        return { quote_id: "q1", record_id: "rec_q1" };
      },
    });

    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" } as CardRow;
    openQuoteForm(anchor, request, "conv1", async () => {});

    const langSel = anchor.querySelector<HTMLSelectElement>(".quote-text-lang")!;
    expect(langSel).not.toBeNull();
    expect(langSel.value).toBe("en");

    // Change to hi
    langSel.value = "hi";

    anchor.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Scope of work";
    anchor.querySelector<HTMLInputElement>(".quote-currency-input")!.value = "EUR";
    anchor.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "100";
    anchor.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "Alice";

    const submitBtn = anchor.querySelector<HTMLButtonElement>(".submit-quote-button")!;
    submitBtn.click();

    await vi.waitFor(() => expect(sentTerms).toBeDefined());
    expect((sentTerms!.scope as Record<string, unknown>).lang).toBe("hi");
  });

  it("readQuoteForm gives the terms submit sends", async () => {
    let sentTerms: Record<string, unknown> | undefined;
    stubRpc({
      "quote.set": (params) => {
        sentTerms = (params as { terms: Record<string, unknown> }).terms;
        return { quote_id: "q1", record_id: "rec_q1" };
      },
    });

    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" } as CardRow;
    openQuoteForm(anchor, request, "conv1", async () => {});

    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Scope of work";
    form.querySelector<HTMLInputElement>(".quote-currency-input")!.value = "EUR";
    form.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "100";
    form.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "Alice";

    const parsed = readQuoteForm(form);
    expect("terms" in parsed).toBe(true);

    const submitBtn = form.querySelector<HTMLButtonElement>(".submit-quote-button")!;
    submitBtn.click();

    await vi.waitFor(() => expect(sentTerms).toBeDefined());
    expect("terms" in parsed && parsed.terms).toEqual(sentTerms);
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
      "enquiry.set": (params) => {
        sentCategories = (params as { categories: string[] }).categories;
        return { enquiry_id: "req-1", record_id: "rec-1" };
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

  it("request form sends lang from its control, starting at UI language", async () => {
    let sentLang: string | undefined;
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
      "enquiry.set": (params) => {
        sentLang = (params as { description?: { lang?: string } }).description?.lang;
        return { enquiry_id: "req-1", record_id: "rec-1" };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const toggleBtn = host.querySelector<HTMLButtonElement>("button.toggle-request-form")!;
    toggleBtn.click();

    const langSel = host.querySelector<HTMLSelectElement>(".request-text-lang")!;
    expect(langSel).not.toBeNull();
    expect(langSel.value).toBe("en");

    langSel.value = "ta";

    const desc = host.querySelector<HTMLTextAreaElement>(".request-desc-input")!;
    desc.value = "Need bike repair";

    const sendBtn = host.querySelector<HTMLButtonElement>("button.send-request-button")!;
    sendBtn.click();

    await vi.waitFor(() => expect(sentLang).toBeDefined());
    expect(sentLang).toBe("ta");
  });

  it("chat box language list sends languages, keeps rows after send, and allows move up and remove", async () => {
    type SendParams = { conversation: string; body: string; languages?: string[] };
    let lastSendParams: SendParams | undefined;
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
      "conversation.send": (params) => {
        lastSendParams = params as SendParams;
        return { message_id: "m_new" };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    // Initial language list has 1 row with English
    let rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("English");

    // First send with default ["en"]
    const composeInput = host.querySelector<HTMLInputElement>(".compose-input")!;
    const sendBtn = Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find(
      (b) => b.textContent === "Send",
    )!;
    composeInput.value = "Hello world";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual(["en"]);
    await vi.waitFor(() => expect(sendBtn.disabled).toBe(false));

    // After send, list keeps its rows
    rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);

    // Add Hindi
    const addSel = host.querySelector<HTMLSelectElement>(".messages-languages-add")!;
    addSel.value = "hi";
    addSel.dispatchEvent(new Event("change"));

    rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("English");
    expect(rows[1].querySelector(".messages-language-name")?.textContent).toBe("Hindi");

    // Move Hindi up
    const upBtn = rows[1].querySelector<HTMLButtonElement>(".messages-language-move-up")!;
    upBtn.click();

    rows = host.querySelectorAll(".messages-language-row");
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("Hindi");
    expect(rows[1].querySelector(".messages-language-name")?.textContent).toBe("English");

    // Send with ["hi", "en"]
    lastSendParams = undefined;
    composeInput.value = "नमस्ते";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual(["hi", "en"]);
    await vi.waitFor(() => expect(sendBtn.disabled).toBe(false));

    // Remove every row
    rows = host.querySelectorAll(".messages-language-row");
    rows[0].querySelector<HTMLButtonElement>(".messages-language-remove")!.click();
    rows = host.querySelectorAll(".messages-language-row");
    rows[0].querySelector<HTMLButtonElement>(".messages-language-remove")!.click();

    expect(host.querySelectorAll(".messages-language-row")).toHaveLength(0);

    // Send with empty list
    lastSendParams = undefined;
    composeInput.value = "No tag message";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual([]);
    await vi.waitFor(() => expect(sendBtn.disabled).toBe(false));
  });

  it("click on language list label or language name does not remove language", async () => {
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
    });

    const host = document.createElement("div");
    await renderMessages(host);

    let rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("English");

    // Click on language name
    const langName = rows[0].querySelector<HTMLElement>(".messages-language-name")!;
    langName.click();
    rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);

    // Click on field label text
    const labelSpan = host.querySelector<HTMLElement>(".compose-row .field > span")!;
    expect(labelSpan.textContent).toBe("Message languages");
    labelSpan.click();
    rows = host.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);
  });

  it("renders lang attribute and note on messages appropriately", async () => {
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 5 }],
      }),
      "transaction.sync": () => ({
        new_records: [
          {
            record_id: "rec_c1",
            card_type: "enquiry",
            version: 1,
            author_did: "peer-1",
            content: { enquiry_id: "req_1", scope: "repair" },
          },
        ],
      }),
      "transaction.thread": () => ({
        cards: [
          {
            message_id: "m_card",
            conversation: "conv-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            card_type: "enquiry",
            version: 1,
            known: true,
            verified: true,
            expired: false,
            record_id: "rec_c1",
            data: { enquiry_id: "req_1" },
            stored_at_secs: 1,
          },
        ],
      }),
      "conversation.history": () => ({
        messages: [
          {
            id: "m_ta",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "text/plain",
            body_encoding: "utf8",
            body: "வணக்கம்",
            languages: ["ta"],
            state: "delivered",
          },
          {
            id: "m_notag",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 2000,
            content_type: "text/plain",
            body_encoding: "utf8",
            body: "plain text without tag",
            state: "delivered",
          },
          {
            id: "m_card",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 3000,
            content_type: "application/vnd.roym.card+json",
            body_encoding: "utf8",
            body: "{}",
            state: "delivered",
          },
          {
            id: "m_del",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 4000,
            content_type: "text/plain",
            body_encoding: "utf8",
            deleted_at_secs: 4000,
            state: "delivered",
          },
          {
            id: "m_bin",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 5000,
            content_type: "application/octet-stream",
            body_encoding: "base64",
            body: "AAAA",
            state: "delivered",
          },
        ],
      }),
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const msgs = host.querySelectorAll(".message");
    expect(msgs.length).toBeGreaterThanOrEqual(5);

    // 1. Tagged text message: lang="ta", no note
    const bodyTa = msgs[0].querySelector(".message-body");
    expect(bodyTa).not.toBeNull();
    expect(bodyTa?.getAttribute("lang")).toBe("ta");
    expect(msgs[0].querySelector(".message-language-note")).toBeNull();

    // 2. Untagged text message: lang="", shows note
    const bodyNoTag = msgs[1].querySelector(".message-body");
    expect(bodyNoTag).not.toBeNull();
    expect(bodyNoTag?.getAttribute("lang")).toBe("");
    const note = msgs[1].querySelector(".message-language-note");
    expect(note).not.toBeNull();
    expect(note?.textContent).toBe("Language not stated");

    // 3. Card message: no note
    expect(msgs[2].querySelector(".message-language-note")).toBeNull();

    // 4. Deleted message: no note
    expect(msgs[3].querySelector(".message-language-note")).toBeNull();

    // 5. Binary message: no note, no lang attribute
    const binBody = msgs[4].querySelector(".message-body");
    expect(binBody).not.toBeNull();
    expect(binBody?.hasAttribute("lang")).toBe(false);
    expect(msgs[4].querySelector(".message-language-note")).toBeNull();
  });

  it("search hit with languages: ['hi'] has snippet with lang='hi'", async () => {
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
      "conversation.search": () => ({
        matches: [
          {
            conversation: "conv-1",
            body: "कुछ खास शब्द खोजें",
            languages: ["hi"],
          },
        ],
      }),
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const sInput = host.querySelector<HTMLInputElement>(".message-search-input")!;
    expect(sInput).not.toBeNull();
    sInput.value = "खास";
    const sBtn = host.querySelector<HTMLButtonElement>(".message-search button")!;
    expect(sBtn).not.toBeNull();
    sBtn.click();

    await vi.waitFor(() => expect(host.querySelector(".search-hit")).not.toBeNull());
    const snippet = host.querySelector<HTMLElement>(".search-hit .snippet")!;
    expect(snippet).not.toBeNull();
    expect(snippet.getAttribute("lang")).toBe("hi");
  });

  it("repeat card is not drawn in the conversation thread", async () => {
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 2 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({
        cards: [
          {
            message_id: "m_card_1",
            conversation: "conv-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            card_type: "enquiry",
            version: 1,
            known: true,
            verified: true,
            expired: false,
            repeat: false,
            record_id: "rec_c1",
            data: { enquiry_id: "req_1" },
            stored_at_secs: 1,
          },
          {
            message_id: "m_card_2",
            conversation: "conv-1",
            direction: "incoming",
            sender_timestamp_ms: 2000,
            card_type: "enquiry",
            version: 1,
            known: true,
            verified: true,
            expired: false,
            repeat: true,
            record_id: "rec_c1",
            data: { enquiry_id: "req_1" },
            stored_at_secs: 2,
          },
        ],
      }),
      "conversation.history": () => ({
        messages: [
          {
            id: "m_card_1",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "application/vnd.roym.card+json",
            body_encoding: "utf8",
            body: "{}",
            state: "delivered",
          },
          {
            id: "m_card_2",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 2000,
            content_type: "application/vnd.roym.card+json",
            body_encoding: "utf8",
            body: "{}",
            state: "delivered",
          },
        ],
      }),
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const msgs = host.querySelectorAll(".message");
    expect(msgs).toHaveLength(1);
    expect(host.querySelector(".message-incoming")).not.toBeNull();
  });

  it("quote card with deal_state accepted hides accept and decline buttons and shows waiting notice", async () => {
    const q: CardRow = {
      message_id: "m_quote",
      conversation: "conv-1",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      card_type: "quote",
      version: 1,
      known: true,
      verified: true,
      expired: false,
      deal_state: "accepted",
      record_id: "rec_q1",
      data: {
        quote_id: "q1",
        counterparty_did: "did:key:me",
        terms: { quote_expires_at_secs: 20000 },
      },
      stored_at_secs: 1,
    };

    const host = document.createElement("div");
    renderCardActions(host, q, "conv-1", "did:key:me", async () => {});

    expect(host.querySelector(".accept-quote-button")).toBeNull();
    expect(host.querySelector(".decline-quote-button")).toBeNull();
    const notice = host.querySelector(".quote-waiting-notice");
    expect(notice).not.toBeNull();
    expect(notice?.textContent).toContain("Waiting for the other side to confirm...");
  });

  it("renderMessages opens conversation with pre-filled listing id and category and empty description", async () => {
    let sentEnquiry: Record<string, unknown> | undefined;
    stubRpc({
      whoami: () => ({ person_did: "did:key:me" }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-bob", peer_address: "did:key:bob", message_count: 0 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({ messages: [] }),
      "enquiry.set": (params) => {
        sentEnquiry = params as Record<string, unknown>;
        return { enquiry_id: "enq-1", record_id: "rec-1" };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host, undefined, {
      conversationId: "conv-bob",
      preFill: { listingId: "lst_gardening_1", categories: ["gardening"] },
    });

    // The thread is rendered and request form is already open
    const reqForm = host.querySelector<HTMLElement>(".request-form")!;
    expect(reqForm).not.toBeNull();
    expect(reqForm.style.display).toBe("block");

    // Listing input is pre-filled with the listing id from the verified envelope
    const listingInput = host.querySelector<HTMLInputElement>(".request-listing-input")!;
    expect(listingInput).not.toBeNull();
    expect(listingInput.value).toBe("lst_gardening_1");

    // Category picker has the listing category pre-selected
    const chip = host.querySelector<HTMLElement>('.request-cats-input .picker-chip[data-code="gardening"]');
    expect(chip).not.toBeNull();

    // Description is empty for the person to type
    const descInput = host.querySelector<HTMLTextAreaElement>(".request-desc-input")!;
    expect(descInput).not.toBeNull();
    expect(descInput.value).toBe("");

    // Person types what they need and submits
    descInput.value = "Need hedge trimming";
    const sendBtn = host.querySelector<HTMLButtonElement>("button.send-request-button")!;
    sendBtn.click();

    await vi.waitFor(() => expect(sentEnquiry).toBeDefined());
    expect(sentEnquiry?.conversation).toBe("conv-bob");
    expect(sentEnquiry?.listing_id).toBe("lst_gardening_1");
    expect(sentEnquiry?.categories).toEqual(["gardening"]);
    expect((sentEnquiry?.description as { text?: string })?.text).toBe("Need hedge trimming");

    // After submit, form is closed and inputs cleared
    await vi.waitFor(() => expect(reqForm.style.display).toBe("none"));
    expect(listingInput.value).toBe("");
    expect(descInput.value).toBe("");
    expect(host.querySelectorAll(".request-cats-input .picker-chip")).toHaveLength(0);
  });
});

describe("widget messages in messages thread", () => {
  const baseMessage = (id: string, contentType: string, body?: string) => ({
    id,
    conversation: "conv-1",
    author: "did:key:alice",
    direction: "incoming" as const,
    sender_timestamp_ms: 1000,
    content_type: contentType,
    body_encoding: "utf8" as const,
    body: body ?? "",
    languages: ["en"],
    state: "delivered" as const,
    stored_at_secs: 1,
  });

  it("a widget message draws the widget card", () => {
    const msg = baseMessage("msg-w1", "application/vnd.roym.widget+json");
    const widgetView: WidgetView = {
      widget_id: "msg-w1",
      author: "did:key:alice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      definition: {
        extension: { id: "poll", version: 1 },
        type: "poll/single",
        semantic: "poll",
        prompt: "Where to eat?",
        fields: [
          {
            name: "option",
            type: "choice",
            required: true,
            choices: [
              { code: "o1", text: "Pizza" },
              { code: "o2", text: "Sushi" },
            ],
          },
        ],
      },
      checked: true,
      counts: [
        { field: "option", choice: "o1", count: 2 },
        { field: "option", choice: "o2", count: 1 },
      ],
      answers: [],
      ignored_answers: 0,
      state: "delivered",
    };
    const widgetMap = new Map<string, WidgetView>([["msg-w1", widgetView]]);

    const el = messageElement(
      msg,
      new Map(),
      [],
      new Map(),
      new Map(),
      "conv-1",
      "did:key:me",
      async () => {},
      widgetMap,
    );

    const card = el.querySelector(".widget-card");
    expect(card).not.toBeNull();
    expect(el.querySelector(".widget-card .message-body")?.textContent).toBe("Where to eat?");
  });

  it("an answer message draws nothing (display none)", () => {
    const msg = baseMessage("msg-a1", "application/vnd.roym.widget-answer+json");
    const el = messageElement(
      msg,
      new Map(),
      [],
      new Map(),
      new Map(),
      "conv-1",
      "did:key:me",
      async () => {},
    );

    expect(el.style.display).toBe("none");
    expect(el.querySelector(".widget-card")).toBeNull();
  });

  it("a widget missing from the map keeps the widget prompt text", () => {
    const msg = baseMessage("msg-w-missing", "application/vnd.roym.widget+json", JSON.stringify({
      widget_version: 1,
      definition: { prompt: "Custom widget prompt" },
    }));
    const el = messageElement(
      msg,
      new Map(),
      [],
      new Map(),
      new Map(),
      "conv-1",
      "did:key:me",
      async () => {},
      new Map(),
    );

    expect(el.querySelector(".widget-refused")).toBeNull();
    expect(el.textContent).toContain("Custom widget prompt");
  });

  it("a widget row has a delete button and retains message state", () => {
    const msg = baseMessage("msg-w-del", "application/vnd.roym.widget+json");
    const el = messageElement(
      msg,
      new Map(),
      [],
      new Map(),
      new Map(),
      "conv-1",
      "did:key:me",
      async () => {},
      new Map(),
    );
    expect(el.querySelector(".delete-message")).not.toBeNull();
    expect(el.querySelector(".message-state")).not.toBeNull();
  });

  it("fillThread calls widget.list only for non-deleted widgets and skips when no widgets exist", async () => {
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "session.whoami": () => ({ did: "did:key:me" }),
      "profile.get": () => ({ profile: null }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 3 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({
        messages: [
          {
            id: "msg-live",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: '{"widget_version":1,"definition":{"prompt":"A live poll"}}',
            state: "delivered",
            deleted_at_secs: null,
          },
          {
            id: "msg-deleted",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1100,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: "",
            state: "delivered",
            deleted_at_secs: 12345,
          },
          {
            id: "msg-txt",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1200,
            content_type: "text/plain",
            body_encoding: "utf8",
            body: "hello",
            state: "delivered",
          },
        ],
      }),
      "widget.list": (params) => {
        calls.push({ method: "widget.list", params });
        return { widgets: [], complete: true };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const widgetListCalls = calls.filter((c) => c.method === "widget.list");
    expect(widgetListCalls).toHaveLength(1);
    expect(widgetListCalls[0].params).toEqual({
      conversation: "conv-1",
      widget_ids: ["msg-live"],
    });
  });

  it("fillThread makes no widget.list call when history has no widgets", async () => {
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "session.whoami": () => ({ did: "did:key:me" }),
      "profile.get": () => ({ profile: null }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 1 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({
        messages: [
          {
            id: "msg-txt",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1200,
            content_type: "text/plain",
            body_encoding: "utf8",
            body: "hello",
            state: "delivered",
          },
        ],
      }),
      "widget.list": (params) => {
        calls.push({ method: "widget.list", params });
        return { widgets: [], complete: true };
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const widgetListCalls = calls.filter((c) => c.method === "widget.list");
    expect(widgetListCalls).toHaveLength(0);
  });

  it("on widget.list error in fillThread, draws a thread-level error and keeps the widget's prompt", async () => {
    stubRpc({
      "session.whoami": () => ({ did: "did:key:me" }),
      "profile.get": () => ({ profile: null }),
      "conversation.list": () => ({
        conversations: [{ id: "conv-1", peer_address: "peer-1", message_count: 1 }],
      }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({
        messages: [
          {
            id: "msg-live",
            conversation: "conv-1",
            author: "peer-1",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: JSON.stringify({
              widget_version: 1,
              definition: { prompt: "Prompt when list fails" },
            }),
            state: "delivered",
          },
        ],
      }),
      "widget.list": () => {
        throw new Error("node restarting");
      },
    });

    const host = document.createElement("div");
    await renderMessages(host);

    const threadErr = host.querySelector(".widget-list-error");
    expect(threadErr).not.toBeNull();
    expect(threadErr?.textContent).toContain("node restarting");

    expect(host.querySelector(".widget-refused")).toBeNull();
    expect(host.textContent).toContain("Prompt when list fails");
  });
});
