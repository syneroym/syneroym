import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { pollComposer } from "./composer";

describe("pollComposer", () => {
  beforeEach(async () => {
    await activate("en");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("starts with 2 empty option rows and disabled send button", () => {
    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    const qInput = comp.form.querySelector<HTMLInputElement>(".widget-poll-question-input")!;
    expect(qInput.value).toBe("");

    const optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    expect(optInputs).toHaveLength(2);
    expect(optInputs[0].value).toBe("");
    expect(optInputs[1].value).toBe("");

    const sendBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-send-btn")!;
    expect(sendBtn.disabled).toBe(true);

    const removeBtns = comp.form.querySelectorAll<HTMLButtonElement>(".widget-poll-remove-option");
    expect(removeBtns).toHaveLength(2);
    expect(removeBtns[0].disabled).toBe(true);
    expect(removeBtns[1].disabled).toBe(true);
  });

  it("enforces between 2 and 12 options, disabling add and remove buttons at bounds", () => {
    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    const addBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-add-option")!;
    expect(addBtn.disabled).toBe(false);

    // Add up to 12 options (already 2, add 10 more)
    for (let i = 0; i < 10; i++) {
      addBtn.click();
    }

    let optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    expect(optInputs).toHaveLength(12);
    expect(addBtn.disabled).toBe(true);

    // Clicking add when at 12 does nothing
    addBtn.click();
    optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    expect(optInputs).toHaveLength(12);

    // Remove down to 2
    for (let i = 0; i < 10; i++) {
      const removeBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-remove-option")!;
      removeBtn.click();
    }

    optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    expect(optInputs).toHaveLength(2);
    const removeBtns = comp.form.querySelectorAll<HTMLButtonElement>(".widget-poll-remove-option");
    expect(removeBtns[0].disabled).toBe(true);
    expect(removeBtns[1].disabled).toBe(true);
  });

  it("disables send button when question is blank or any option is blank", () => {
    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    const qInput = comp.form.querySelector<HTMLInputElement>(".widget-poll-question-input")!;
    const sendBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-send-btn")!;
    const hint = comp.form.querySelector<HTMLParagraphElement>(".widget-poll-composer-hint")!;

    qInput.value = "Favorite color?";
    qInput.dispatchEvent(new Event("input"));
    expect(sendBtn.disabled).toBe(true);
    expect(hint.textContent).toBe("A poll needs at least two options.");

    const optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    optInputs[0].value = "Red";
    optInputs[0].dispatchEvent(new Event("input"));
    expect(sendBtn.disabled).toBe(true);
    expect(hint.textContent).toBe("A poll needs at least two options.");

    optInputs[1].value = "Blue";
    optInputs[1].dispatchEvent(new Event("input"));
    expect(sendBtn.disabled).toBe(false);
    expect(hint.textContent).toBe("");
  });

  it("disables send button and shows hint when options are duplicated case-insensitively", () => {
    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    const qInput = comp.form.querySelector<HTMLInputElement>(".widget-poll-question-input")!;
    const sendBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-send-btn")!;
    const hint = comp.form.querySelector<HTMLParagraphElement>(".widget-poll-composer-hint")!;

    qInput.value = "Favorite color?";
    qInput.dispatchEvent(new Event("input"));

    const optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    optInputs[0].value = "Red";
    optInputs[0].dispatchEvent(new Event("input"));
    optInputs[1].value = "red";
    optInputs[1].dispatchEvent(new Event("input"));

    expect(sendBtn.disabled).toBe(true);
    expect(hint.textContent).toBe("Options must be different.");
  });

  it("calls widget.send with pollDefinition and resets form on success", async () => {
    let sentParams: Record<string, unknown> | undefined;
    stubRpc({
      "widget.send": (params) => {
        sentParams = params as Record<string, unknown>;
        return { message_id: "msg-poll-1" };
      },
    });

    let onSentCalled = false;
    const comp = pollComposer({
      conversation: "conv-42",
      languages: () => ["en", "hi"],
      onSent: () => {
        onSentCalled = true;
      },
    });

    const qInput = comp.form.querySelector<HTMLInputElement>(".widget-poll-question-input")!;
    qInput.value = "Lunch spot?";
    qInput.dispatchEvent(new Event("input"));

    const optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    optInputs[0].value = "Diner";
    optInputs[0].dispatchEvent(new Event("input"));
    optInputs[1].value = "Cafe";
    optInputs[1].dispatchEvent(new Event("input"));

    const sendBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-send-btn")!;
    sendBtn.click();

    await vi.waitFor(() => expect(onSentCalled).toBe(true));
    expect(sentParams).toBeDefined();
    expect(sentParams?.conversation).toBe("conv-42");
    expect(sentParams?.languages).toEqual(["en", "hi"]);
    expect(sentParams?.widget).toEqual({
      extension: { id: "poll", version: 1 },
      type: "poll/single",
      semantic: "poll",
      prompt: "Lunch spot?",
      fields: [
        {
          name: "option",
          type: "choice",
          required: true,
          choices: [
            { code: "o1", text: "Diner" },
            { code: "o2", text: "Cafe" },
          ],
        },
      ],
    });

    expect(qInput.value).toBe("");
    expect(comp.form.style.display).toBe("none");
  });

  it("shows error message when widget.send fails", async () => {
    stubRpc({
      "widget.send": () => {
        throw new Error("network down");
      },
    });

    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    const qInput = comp.form.querySelector<HTMLInputElement>(".widget-poll-question-input")!;
    qInput.value = "Question?";
    qInput.dispatchEvent(new Event("input"));

    const optInputs = comp.form.querySelectorAll<HTMLInputElement>(".widget-poll-option-input");
    optInputs[0].value = "Yes";
    optInputs[0].dispatchEvent(new Event("input"));
    optInputs[1].value = "No";
    optInputs[1].dispatchEvent(new Event("input"));

    const sendBtn = comp.form.querySelector<HTMLButtonElement>(".widget-poll-send-btn")!;
    sendBtn.click();

    const hint = comp.form.querySelector<HTMLParagraphElement>(".widget-poll-composer-hint")!;
    await vi.waitFor(() => expect(hint.textContent).toContain("network down"));
    expect(sendBtn.disabled).toBe(false);
  });

  it("toggle button toggles form display between block and none", () => {
    const comp = pollComposer({
      conversation: "conv-1",
      languages: () => ["en"],
    });

    expect(comp.form.style.display).toBe("none");
    comp.button.click();
    expect(comp.form.style.display).toBe("block");
    comp.button.click();
    expect(comp.form.style.display).toBe("none");
  });
});
