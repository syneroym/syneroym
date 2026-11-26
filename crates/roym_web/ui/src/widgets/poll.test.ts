import { describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import { renderPoll } from "./poll";
import { pollDefinition, type WidgetView } from "./view";

function makePollView(overrides: Partial<WidgetView> = {}): WidgetView {
  return {
    widget_id: "w1",
    author: "did:key:alice",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    languages: ["en"],
    definition: pollDefinition("Which day shall we meet?", ["Saturday", "Sunday"]),
    refused: null,
    checked: true,
    counts: [
      { field: "option", choice: "o1", count: 0 },
      { field: "option", choice: "o2", count: 0 },
    ],
    answers: [],
    ignored_answers: 0,
    state: "delivered",
    last_error: null,
    ...overrides,
  };
}

describe("renderPoll", () => {
  it("renders heading from catalog for known type, else generic code", () => {
    const knownView = makePollView();
    const elKnown = renderPoll(knownView, { conversation: "c1" });
    const h3Known = elKnown.querySelector(".widget-heading");
    expect(h3Known?.textContent).toBe("Poll");

    const unknownView = makePollView({
      checked: false,
      definition: {
        extension: { id: "custom", version: 1 },
        type: "custom/poll",
        semantic: "poll",
        prompt: "Vote here",
        fields: [
          {
            name: "choice",
            type: "choice",
            required: true,
            choices: [
              { code: "c1", text: "Yes" },
              { code: "c2", text: "No" },
            ],
          },
        ],
      },
    });
    const elUnknown = renderPoll(unknownView, { conversation: "c1" });
    const h3Unknown = elUnknown.querySelector(".widget-heading");
    expect(h3Unknown?.textContent).toContain("custom/poll");
  });

  it("draws ext-not-checked when checked is false", () => {
    const viewUnchecked = makePollView({ checked: false });
    const el = renderPoll(viewUnchecked, { conversation: "c1" });
    const notChecked = el.querySelector(".widget-not-checked");
    expect(notChecked).not.toBeNull();
    expect(notChecked?.textContent).toBe("Not checked on this device");

    const viewChecked = makePollView({ checked: true });
    const elChecked = renderPoll(viewChecked, { conversation: "c1" });
    expect(elChecked.querySelector(".widget-not-checked")).toBeNull();
  });

  it("renders zero counts initially", () => {
    const view = makePollView();
    const el = renderPoll(view, { conversation: "c1" });
    const counts = el.querySelectorAll(".poll-option-count");
    expect(counts).toHaveLength(2);
    expect(counts[0].textContent).toBe("0 votes");
    expect(counts[1].textContent).toBe("0 votes");
  });

  it("shows the person's own choice", () => {
    const view = makePollView({
      counts: [
        { field: "option", choice: "o1", count: 0 },
        { field: "option", choice: "o2", count: 1 },
      ],
      answers: [
        {
          message_id: "ans-1",
          author: "did:key:bob",
          direction: "outgoing",
          sender_timestamp_ms: 1200,
          languages: ["en"],
          data: { option: "o2" },
          state: "delivered",
        },
      ],
    });
    const el = renderPoll(view, { conversation: "c1" });
    const yourChoice = el.querySelector(".widget-poll-your-choice");
    expect(yourChoice?.textContent).toBe("You chose: Sunday");
    const selectedBtn = el.querySelector(".widget-poll-option-btn.selected");
    expect(selectedBtn?.textContent).toContain("Sunday");
  });

  it("a tap calls widget.answer with field name derived from definition", async () => {
    const view = makePollView({
      definition: {
        extension: { id: "poll", version: 1 },
        type: "poll/single",
        semantic: "poll",
        prompt: "Choose",
        fields: [
          {
            name: "custom_vote",
            type: "choice",
            required: true,
            choices: [
              { code: "o1", text: "Alpha" },
              { code: "o2", text: "Beta" },
            ],
          },
        ],
      },
    });
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "widget.answer": (params) => {
        calls.push({ method: "widget.answer", params });
        return Promise.resolve({ ok: true });
      },
    });
    const refresh = vi.fn();
    const el = renderPoll(view, { conversation: "conv-99", refresh });
    const buttons = el.querySelectorAll<HTMLButtonElement>(".widget-poll-option-btn");
    buttons[1].click();

    await vi.waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      method: "widget.answer",
      params: {
        conversation: "conv-99",
        widget_id: "w1",
        data: { custom_vote: "o2" },
        languages: [],
      },
    });
  });

  it("passes composer languages to widget.answer", async () => {
    const view = makePollView();
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "widget.answer": (params) => {
        calls.push({ method: "widget.answer", params });
        return Promise.resolve({ ok: true });
      },
    });
    const el = renderPoll(view, {
      conversation: "conv-99",
      languages: () => ["hi", "en"],
    });
    const buttons = el.querySelectorAll<HTMLButtonElement>(".widget-poll-option-btn");
    buttons[0].click();

    await vi.waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual({
      method: "widget.answer",
      params: {
        conversation: "conv-99",
        widget_id: "w1",
        data: { option: "o1" },
        languages: ["hi", "en"],
      },
    });
  });

  it("disables all option buttons while call runs, and displays error when answer is refused", async () => {
    let rejectCall: ((err: Error) => void) | null = null;
    stubRpc({
      "widget.answer": () =>
        new Promise((_, reject) => {
          rejectCall = reject;
        }),
    });
    const view = makePollView();
    const el = renderPoll(view, { conversation: "conv-99" });
    const buttons = el.querySelectorAll<HTMLButtonElement>(".widget-poll-option-btn");
    expect(buttons[0].disabled).toBe(false);
    expect(buttons[1].disabled).toBe(false);

    buttons[0].click();
    expect(buttons[0].disabled).toBe(true);
    expect(buttons[1].disabled).toBe(true);

    rejectCall!(new Error("vote refused by node"));
    await vi.waitFor(() => expect(buttons[0].disabled).toBe(false));
    expect(buttons[1].disabled).toBe(false);

    const errP = el.querySelector(".widget-answer-error");
    expect(errP).not.toBeNull();
    expect(errP?.textContent).toContain("vote refused by node");
  });

  it("a tap on the current choice makes no call", async () => {
    const view = makePollView({
      answers: [
        {
          message_id: "ans-1",
          author: "did:key:bob",
          direction: "outgoing",
          sender_timestamp_ms: 1200,
          languages: ["en"],
          data: { option: "o1" },
          state: "delivered",
        },
      ],
    });
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "widget.answer": (params) => {
        calls.push({ method: "widget.answer", params });
        return Promise.resolve({ ok: true });
      },
    });
    const refresh = vi.fn();
    const el = renderPoll(view, { conversation: "conv-99", refresh });
    const buttons = el.querySelectorAll<HTMLButtonElement>(".widget-poll-option-btn");
    buttons[0].click(); // current choice

    expect(calls).toEqual([]);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("draws choice text with markup as plain text", () => {
    const markupText = "<b onclick=alert(1)>Saturday</b>";
    const view = makePollView({
      definition: pollDefinition("Question", [markupText, "Sunday"]),
    });
    const el = renderPoll(view, { conversation: "c1" });
    const textSpan = el.querySelector(".poll-option-text");
    expect(textSpan?.querySelector("b")).toBeNull();
    expect(textSpan?.textContent).toBe(markupText);
  });

  it("sets the lang attribute on option text from view languages", () => {
    const view = makePollView({ languages: ["hi", "en"] });
    const el = renderPoll(view, { conversation: "c1" });
    const textSpan = el.querySelector(".poll-option-text");
    expect(textSpan?.getAttribute("lang")).toBe("hi");
  });

  it("does not draw widget-level state word or retry button on card (row owns it)", () => {
    const view = makePollView({
      direction: "outgoing",
      state: "failed",
      last_error: "network timeout",
    });
    const el = renderPoll(view, { conversation: "c1" });
    expect(el.querySelector(".widget-state")).toBeNull();
    expect(el.querySelector(".retry-widget")).toBeNull();
  });

  it("draws state word and retry button when an own answer has state failed", async () => {
    const view = makePollView({
      answers: [
        {
          message_id: "ans-fail-1",
          author: "did:key:bob",
          direction: "outgoing",
          sender_timestamp_ms: 1200,
          languages: ["en"],
          data: { option: "o1" },
          state: "failed",
          last_error: "peer unavailable",
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
    const el = renderPoll(view, { conversation: "c1", refresh });

    const stateWord = el.querySelector(".answer-state");
    expect(stateWord?.textContent).toBe("failed");
    const errorSpan = el.querySelector(".answer-error");
    expect(errorSpan?.textContent).toBe("peer unavailable");

    const retryBtn = el.querySelector<HTMLButtonElement>(".retry-answer");
    expect(retryBtn).not.toBeNull();
    retryBtn?.click();

    await vi.waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      method: "conversation.retry",
      params: { message_id: "ans-fail-1" },
    });
  });

  it("suppresses retry button for restored failed own answer", () => {
    const view = makePollView({
      answers: [
        {
          message_id: "ans-restored-1",
          author: "did:key:bob",
          direction: "outgoing",
          sender_timestamp_ms: 1200,
          languages: ["en"],
          data: { option: "o1" },
          state: "failed",
          last_error: "peer unavailable",
        },
      ],
    });
    const el = renderPoll(view, {
      conversation: "c1",
      isRestored: (id) => id === "ans-restored-1",
    });

    const stateWord = el.querySelector(".answer-state");
    expect(stateWord?.textContent).toBe("failed");
    expect(el.querySelector(".retry-answer")).toBeNull();
  });
});
