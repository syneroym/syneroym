import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { activate } from "../i18n/i18n";
import { categoryPicker } from "./category_picker";
import { CATEGORY_GROUPS, categoryLabel } from "./taxonomy";

describe("categoryPicker component", () => {
  beforeEach(async () => {
    await activate("en");
  });

  afterEach(async () => {
    await activate("en");
  });

  it("shows every group as a heading and its children as options when filter is empty", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const groups = picker.root.querySelectorAll(".picker-group");
    expect(groups.length).toBe(CATEGORY_GROUPS.length);

    const firstGroupHeading = groups[0];
    expect(firstGroupHeading.textContent).toBe(categoryLabel(CATEGORY_GROUPS[0].code));

    const options = picker.root.querySelectorAll(".picker-option");
    const totalChildren = CATEGORY_GROUPS.reduce((acc, g) => acc + g.children.length, 0);
    expect(options.length).toBe(totalChildren);
  });

  it("filters options by typing, adds chip on click, returns code, and removes via remove button", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    filter.value = "Plumbing";
    filter.dispatchEvent(new Event("input"));

    const options = picker.root.querySelectorAll<HTMLButtonElement>(".picker-option");
    expect(options.length).toBe(1);
    expect(options[0].dataset.code).toBe("plumbing");

    options[0].click();

    expect(picker.codes()).toEqual(["plumbing"]);
    const chips = picker.root.querySelectorAll<HTMLLIElement>(".picker-chip");
    expect(chips.length).toBe(1);
    expect(chips[0].dataset.code).toBe("plumbing");
    expect(chips[0].textContent).toContain("Plumbing");

    const removeBtn = chips[0].querySelector<HTMLButtonElement>(".picker-chip-remove")!;
    removeBtn.click();

    expect(picker.codes()).toEqual([]);
    expect(picker.root.querySelectorAll(".picker-chip").length).toBe(0);
  });

  it("does not offer a chosen code again; disables options and shows full message at max", () => {
    const picker = categoryPicker({ className: "test-picker", max: 2 });

    const btn1 = picker.root.querySelector<HTMLButtonElement>('.picker-option[data-code="plumbing"]')!;
    btn1.click();

    expect(picker.root.querySelector('.picker-option[data-code="plumbing"]')).toBeNull();

    const btn2 = picker.root.querySelector<HTMLButtonElement>('.picker-option[data-code="electrical"]')!;
    btn2.click();

    expect(picker.codes()).toEqual(["plumbing", "electrical"]);

    const message = picker.root.querySelector(".picker-message")!;
    expect(message.textContent).toContain("up to 2 categories");

    const allOptions = picker.root.querySelectorAll<HTMLButtonElement>(".picker-option");
    for (const opt of allOptions) {
      expect(opt.disabled).toBe(true);
    }
  });

  it("handles own code, duplicate own code, taxonomy label, and invalid code", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    // A valid own code not in taxonomy
    filter.value = "my-group-code";
    filter.dispatchEvent(new Event("input"));

    const ownBtn = picker.root.querySelector<HTMLButtonElement>(".picker-own-code");
    expect(ownBtn).not.toBeNull();
    expect(ownBtn!.dataset.code).toBe("my-group-code");
    expect(ownBtn!.textContent).toContain("my-group-code");

    ownBtn!.click();
    expect(picker.codes()).toEqual(["my-group-code"]);

    const chip = picker.root.querySelector<HTMLLIElement>(".picker-chip[data-code='my-group-code']")!;
    expect(chip.textContent).toContain("my-group-code");

    // Typing the same own code again does not offer it
    filter.value = "my-group-code";
    filter.dispatchEvent(new Event("input"));
    expect(picker.root.querySelector(".picker-own-code")).toBeNull();

    // "Plumbing" (taxonomy label) offers no own-code option
    filter.value = "Plumbing";
    filter.dispatchEvent(new Event("input"));
    expect(picker.root.querySelector(".picker-own-code")).toBeNull();

    // "bad code!" offers no own-code option
    filter.value = "bad code!";
    filter.dispatchEvent(new Event("input"));
    expect(picker.root.querySelector(".picker-own-code")).toBeNull();
  });

  it("shows no-match message when filter has text and there are no options", async () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;
    const message = picker.root.querySelector(".picker-message")!;

    filter.value = "no match at all!";
    filter.dispatchEvent(new Event("input"));
    expect(message.textContent).toBe("No category matches.");

    await activate("hi");
    filter.dispatchEvent(new Event("input"));
    expect(message.textContent).not.toBe("No category matches.");
    expect(/[ऀ-ॿ]/.test(message.textContent ?? "")).toBe(true);
  });

  it("empties the chips and codes() via clear()", () => {
    const picker = categoryPicker({
      className: "test-picker",
      max: 8,
      selected: ["plumbing", "electrical"],
    });
    expect(picker.codes()).toEqual(["plumbing", "electrical"]);
    expect(picker.root.querySelectorAll(".picker-chip").length).toBe(2);

    picker.clear();
    expect(picker.codes()).toEqual([]);
    expect(picker.root.querySelectorAll(".picker-chip").length).toBe(0);
  });

  it("options are hidden until the filter has focus, and focusout hides them", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const options = picker.root.querySelector<HTMLDivElement>(".picker-options")!;
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    expect(options.hidden).toBe(true);

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(options.hidden).toBe(false);

    filter.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    expect(options.hidden).toBe(true);
  });

  it("clear() and Escape hide options", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const options = picker.root.querySelector<HTMLDivElement>(".picker-options")!;
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(options.hidden).toBe(false);

    picker.clear();
    expect(options.hidden).toBe(true);

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(options.hidden).toBe(false);

    filter.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(options.hidden).toBe(true);
  });

  it("Tab from the filter with no text closes options so focus reaches the next field", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const options = picker.root.querySelector<HTMLDivElement>(".picker-options")!;
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(options.hidden).toBe(false);

    filter.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(options.hidden).toBe(true);
  });

  it("picks the option on Enter when exactly one visible option exists", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;

    filter.value = "Plumbing";
    filter.dispatchEvent(new Event("input"));

    const options = picker.root.querySelectorAll(".picker-option");
    expect(options.length).toBe(1);

    filter.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));

    expect(picker.codes()).toEqual(["plumbing"]);
    expect(filter.value).toBe("");
  });

  it("has aria-live='polite' on the message container", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const message = picker.root.querySelector(".picker-message")!;
    expect(message.getAttribute("aria-live")).toBe("polite");
  });

  it("shows taxonomy label and raw code for initial selected array", () => {
    const picker = categoryPicker({
      className: "test-picker",
      max: 8,
      selected: ["plumbing", "own-custom-code"],
    });
    expect(picker.codes()).toEqual(["plumbing", "own-custom-code"]);

    const chips = picker.root.querySelectorAll<HTMLLIElement>(".picker-chip");
    expect(chips.length).toBe(2);
    expect(chips[0].textContent).toContain("Plumbing");
    expect(chips[1].textContent).toContain("own-custom-code");
  });

  it("prevents mousedown on options to preserve focus and allow clicking an option", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;
    const optionsContainer = picker.root.querySelector<HTMLDivElement>(".picker-options")!;

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(optionsContainer.hidden).toBe(false);

    const option = picker.root.querySelector<HTMLButtonElement>('.picker-option[data-code="plumbing"]')!;
    const mousedownEvent = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    option.dispatchEvent(mousedownEvent);
    expect(mousedownEvent.defaultPrevented).toBe(true);

    option.click();
    expect(picker.codes()).toEqual(["plumbing"]);
  });

  it("closes options after a pick, and clicking filter re-opens them", () => {
    const picker = categoryPicker({ className: "test-picker", max: 8 });
    const filter = picker.root.querySelector<HTMLInputElement>(".picker-filter")!;
    const options = picker.root.querySelector<HTMLDivElement>(".picker-options")!;

    filter.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(options.hidden).toBe(false);

    const option = picker.root.querySelector<HTMLButtonElement>('.picker-option[data-code="plumbing"]')!;
    option.click();

    expect(picker.codes()).toEqual(["plumbing"]);
    expect(options.hidden).toBe(true);

    filter.click();
    expect(options.hidden).toBe(false);
  });

  it("shows Hindi labels for chips and options in Hindi", async () => {
    await activate("hi");
    const picker = categoryPicker({
      className: "test-picker",
      max: 8,
      selected: ["plumbing"],
    });

    const chip = picker.root.querySelector<HTMLLIElement>(".picker-chip")!;
    expect(/[ऀ-ॿ]/.test(chip.textContent ?? "")).toBe(true);

    const firstGroupHeading = picker.root.querySelector(".picker-group")!;
    expect(/[ऀ-ॿ]/.test(firstGroupHeading.textContent ?? "")).toBe(true);
  });
});
