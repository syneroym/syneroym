import { describe, expect, it } from "vitest";
import { renderWidget } from "./render";
import { pollDefinition, type WidgetView } from "./view";

function makeBaseView(overrides: Partial<WidgetView> = {}): WidgetView {
  return {
    widget_id: "w-render-1",
    author: "did:key:alice",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    languages: ["en"],
    definition: pollDefinition("Question", ["A", "B"]),
    refused: null,
    checked: true,
    counts: [],
    answers: [],
    ignored_answers: 0,
    state: "delivered",
    last_error: null,
    ...overrides,
  };
}

describe("renderWidget", () => {
  it("picks renderPoll when semantic is poll and exactly one required choice field", () => {
    const pollView = makeBaseView();
    const el = renderWidget(pollView, { conversation: "c1" });
    expect(el.classList.contains("widget-poll")).toBe(true);
    expect(el.classList.contains("widget-generic")).toBe(false);
  });

  it("picks renderGenericWidget when definition is not a single required choice poll", () => {
    const genericView = makeBaseView({
      definition: {
        extension: { id: "custom", version: 1 },
        type: "custom/form",
        semantic: "form",
        prompt: "Fill form",
        fields: [{ name: "notes", type: "text" }],
      },
    });
    const el = renderWidget(genericView, { conversation: "c1" });
    expect(el.classList.contains("widget-generic")).toBe(true);
    expect(el.classList.contains("widget-poll")).toBe(false);
  });

  it("draws the two refused texts: widget-too-large and widget-refused with reason", () => {
    // 1. widget-too-large
    const largeView = makeBaseView({
      refused: "widget-too-large",
      definition: null,
    });
    const elLarge = renderWidget(largeView, { conversation: "c1" });
    expect(elLarge.classList.contains("widget-refused")).toBe(true);
    expect(elLarge.querySelector(".refused-notice")?.textContent).toBe(
      "This widget is too large for this app.",
    );

    // 2. other refusal code
    const shapeView = makeBaseView({
      refused: "widget-shape",
      definition: null,
    });
    const elShape = renderWidget(shapeView, { conversation: "c1" });
    expect(elShape.classList.contains("widget-refused")).toBe(true);
    expect(elShape.querySelector(".refused-notice")?.textContent).toBe(
      "This widget cannot be shown: widget-shape",
    );
  });

  it("renders normally when complete is false", () => {
    // The Hub ignores complete: false and renders the counts it has
    const incompleteView = makeBaseView({
      counts: [
        { field: "option", choice: "o1", count: 5 },
        { field: "option", choice: "o2", count: 3 },
      ],
    });
    expect(() => {
      const el = renderWidget(incompleteView, { conversation: "c1" });
      expect(el.classList.contains("widget-poll")).toBe(true);
      const counts = el.querySelectorAll(".poll-option-count");
      expect(counts[0].textContent).toBe("5 votes");
      expect(counts[1].textContent).toBe("3 votes");
    }).not.toThrow();
  });
});
