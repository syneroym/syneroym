import { afterEach, describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import { ENGINE, fakeEngine, readerReads, unavailableEngine } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { renderGenericWidget } from "./generic";
import type { WidgetDefinition, WidgetView } from "./view";

function makeGenericView(overrides: Partial<WidgetView> = {}): WidgetView {
  const definition: WidgetDefinition = {
    extension: { id: "custom", version: 1 },
    type: "custom/form",
    semantic: "form",
    prompt: "Please fill out this form",
    fields: [
      { name: "f_text", type: "text", required: true, max_len: 50, label: "Your name" },
      { name: "f_code", type: "code", max_len: 20 },
      { name: "f_long", type: "long-text", max_len: 200, label: "Notes" },
      { name: "f_num", type: "number", min: 1, max: 100 },
      { name: "f_date", type: "date-time" },
      { name: "f_dur", type: "duration" },
      {
        name: "f_choice",
        type: "choice",
        choices: [
          { code: "c1", text: "First" },
          { code: "c2", text: "Second" },
        ],
      },
    ],
  };

  return {
    widget_id: "w-gen-1",
    author: "did:key:alice",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    languages: ["en"],
    definition,
    refused: null,
    checked: false,
    counts: [],
    answers: [],
    ignored_answers: 0,
    state: "delivered",
    last_error: null,
    ...overrides,
  };
}

describe("renderGenericWidget", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    setTargetsForTest([]);
  });

  it("draws fields of an unknown type and 'Not checked on this device'", () => {
    const view = makeGenericView({ checked: false });
    const el = renderGenericWidget(view, { conversation: "c1" });
    const notChecked = el.querySelector(".widget-not-checked");
    expect(notChecked).not.toBeNull();
    expect(notChecked?.textContent).toBe("Not checked on this device");
    expect(el.querySelector(".widget-heading")?.textContent).toContain("custom/form");
  });

  it("renders an input for each field type", () => {
    const view = makeGenericView();
    const el = renderGenericWidget(view, { conversation: "c1" });

    const textInput = el.querySelector<HTMLInputElement>('input[name="f_text"]');
    expect(textInput?.type).toBe("text");
    expect(textInput?.maxLength).toBe(50);

    const codeInput = el.querySelector<HTMLInputElement>('input[name="f_code"]');
    expect(codeInput?.type).toBe("text");
    expect(codeInput?.maxLength).toBe(20);

    const longText = el.querySelector<HTMLTextAreaElement>('textarea[name="f_long"]');
    expect(longText?.maxLength).toBe(200);

    const numInput = el.querySelector<HTMLInputElement>('input[name="f_num"]');
    expect(numInput?.type).toBe("number");
    expect(numInput?.min).toBe("1");
    expect(numInput?.max).toBe("100");

    const dateInput = el.querySelector<HTMLInputElement>('input[name="f_date"]');
    expect(dateInput?.type).toBe("datetime-local");

    const durInput = el.querySelector<HTMLInputElement>('input[name="f_dur"]');
    expect(durInput?.type).toBe("number");

    const select = el.querySelector<HTMLSelectElement>('select[name="f_choice"]');
    expect(select?.options).toHaveLength(2);
    expect(select?.options[0].value).toBe("c1");
    expect(select?.options[0].textContent).toBe("First");
  });

  it("labels fields by catalog for known type, then author label, then field name", () => {
    // 1. Author label
    const viewAuthor = makeGenericView({
      definition: {
        extension: { id: "custom", version: 1 },
        type: "custom/form",
        semantic: "form",
        prompt: "Prompt",
        fields: [{ name: "my_field", type: "text", label: "Custom Author Label" }],
      },
    });
    const elAuthor = renderGenericWidget(viewAuthor, { conversation: "c1" });
    expect(elAuthor.textContent).toContain("Custom Author Label");

    // 2. Field name fallback
    const viewName = makeGenericView({
      definition: {
        extension: { id: "custom", version: 1 },
        type: "custom/form",
        semantic: "form",
        prompt: "Prompt",
        fields: [{ name: "bare_field", type: "text" }],
      },
    });
    const elName = renderGenericWidget(viewName, { conversation: "c1" });
    expect(elName.textContent).toContain("bare_field");

    // 3. Known type from catalog
    const viewCatalog = makeGenericView({
      checked: true,
      definition: {
        extension: { id: "poll", version: 1 },
        type: "poll/single",
        semantic: "poll",
        prompt: "Prompt",
        fields: [{ name: "option", type: "choice" }],
      },
    });
    const elCatalog = renderGenericWidget(viewCatalog, { conversation: "c1" });
    expect(elCatalog.textContent).toContain("Your choice");
  });

  it("submits an answer when Send answer is tapped", async () => {
    const view = makeGenericView();
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "widget.answer": (params) => {
        calls.push({ method: "widget.answer", params });
        return Promise.resolve({ ok: true });
      },
    });
    const refresh = vi.fn();
    const el = renderGenericWidget(view, { conversation: "c-123", refresh });

    const textInput = el.querySelector<HTMLInputElement>('input[name="f_text"]')!;
    textInput.value = "John Doe";

    const numInput = el.querySelector<HTMLInputElement>('input[name="f_num"]')!;
    numInput.value = "42";

    const submitBtn = el.querySelector<HTMLButtonElement>(".widget-answer-submit")!;
    submitBtn.click();

    await vi.waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      method: "widget.answer",
      params: {
        conversation: "c-123",
        widget_id: "w-gen-1",
        data: expect.objectContaining({
          f_text: "John Doe",
          f_num: 42,
        }),
      },
    });
  });

  it("draws state word and retry button for own widget or own answer with state failed", async () => {
    const view = makeGenericView({
      direction: "outgoing",
      state: "failed",
      last_error: "widget failed to send",
      answers: [
        {
          message_id: "ans-gen-1",
          author: "did:key:bob",
          direction: "outgoing",
          sender_timestamp_ms: 1100,
          languages: ["en"],
          data: { f_text: "Val" },
          state: "failed",
          last_error: "answer write error",
        },
      ],
    });
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "conversation.retry": (params) => {
        calls.push({ method: "conversation.retry", params });
        return Promise.resolve({ ok: true });
      },
    });
    const refresh = vi.fn();
    const el = renderGenericWidget(view, { conversation: "c1", refresh });

    expect(el.querySelector(".widget-state")?.textContent).toBe("failed");
    expect(el.querySelector(".widget-error")?.textContent).toBe("widget failed to send");

    expect(el.querySelector(".answer-state")?.textContent).toBe("failed");
    expect(el.querySelector(".answer-error")?.textContent).toBe("answer write error");

    const retryBtns = el.querySelectorAll<HTMLButtonElement>(".retry-message");
    expect(retryBtns).toHaveLength(2);

    retryBtns[0].click(); // retry widget
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0].params).toEqual({ message_id: "w-gen-1" });

    retryBtns[1].click(); // retry answer
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    expect(calls[1].params).toEqual({ message_id: "ans-gen-1" });
  });

  it("renders the answers list under the form", () => {
    const view = makeGenericView({
      answers: [
        {
          message_id: "ans-1",
          author: "did:key:carol",
          direction: "incoming",
          sender_timestamp_ms: 2000,
          languages: ["en"],
          data: {
            f_text: "Hello there",
            f_num: 99,
          },
          state: "delivered",
        },
      ],
    });
    const el = renderGenericWidget(view, { conversation: "c1", nameOf: (addr) => `Carol (${addr})` });
    const answersSection = el.querySelector(".widget-answers-section");
    expect(answersSection).not.toBeNull();
    expect(answersSection?.querySelector(".widget-answers-heading")?.textContent).toBe("Answers");

    const authorSpan = answersSection?.querySelector(".answer-author");
    expect(authorSpan?.textContent).toBe("Carol (did:key:carol)");

    const textLine = answersSection?.querySelector(".answer-text-line");
    expect(textLine?.textContent).toContain("Your name: Hello there");
  });

  describe("widget-answer hook point (ML-26)", () => {
    it("a node with no engine draws no translate action and makes no call", () => {
      readerReads([]);
      setTargetsForTest([]);
      const calls = stubRpc({});
      const view = makeGenericView({
        answers: [
          {
            message_id: "ans-hi-1",
            author: "did:key:dan",
            direction: "incoming",
            sender_timestamp_ms: 2100,
            languages: ["hi"],
            data: { f_text: "नमस्ते" },
            state: "delivered",
          },
        ],
      });
      const el = renderGenericWidget(view, { conversation: "c1" });
      expect(el.querySelector(".message-translate")).toBeNull();
      expect(calls).toEqual([]);
    });

    it("translates on click with exact translation.translate params", async () => {
      readerReads([]);
      setTargetsForTest(["en"]);
      const seen: Record<string, unknown>[] = [];
      stubRpc({ "translation.translate": fakeEngine(seen) });

      const view = makeGenericView({
        answers: [
          {
            message_id: "ans-hi-2",
            author: "did:key:dan",
            direction: "incoming",
            sender_timestamp_ms: 2200,
            languages: ["hi"],
            data: { f_text: "नमस्ते" },
            state: "delivered",
          },
        ],
      });
      const el = renderGenericWidget(view, { conversation: "c1" });
      const button = el.querySelector<HTMLButtonElement>(".message-translate")!;
      expect(button).not.toBeNull();
      expect(button.textContent).toBe("Translate into English");

      button.click();
      await vi.waitFor(() => expect(el.querySelector(".message-translation")).not.toBeNull());

      expect(seen).toEqual([{ texts: ["नमस्ते"], source_lang: "hi", target_lang: "en" }]);
      const translation = el.querySelector(".message-translation")!;
      expect([translation.textContent, translation.getAttribute("lang")]).toEqual(["T:नमस्ते", "en"]);
      expect(el.querySelector(".message-translation-note")!.textContent).toBe(
        `Machine translation into English by ${ENGINE}.`,
      );
    });

    it("keeps original text and shows error when translation engine is unavailable", async () => {
      readerReads([]);
      setTargetsForTest(["en"]);
      stubRpc({ "translation.translate": unavailableEngine });

      const view = makeGenericView({
        answers: [
          {
            message_id: "ans-hi-3",
            author: "did:key:dan",
            direction: "incoming",
            sender_timestamp_ms: 2300,
            languages: ["hi"],
            data: { f_text: "नमस्ते" },
            state: "delivered",
          },
        ],
      });
      const el = renderGenericWidget(view, { conversation: "c1" });
      const button = el.querySelector<HTMLButtonElement>(".message-translate")!;
      button.click();

      await vi.waitFor(() => expect(el.querySelector(".message-translation-error")).not.toBeNull());
      expect(el.querySelector(".message-translation-error")!.textContent).toBe(
        "Translation is not available.",
      );
      expect(el.querySelector(".answer-text-line .message-body")!.textContent).toBe("नमस्ते");
      expect(button.disabled).toBe(false);
    });
  });
});
