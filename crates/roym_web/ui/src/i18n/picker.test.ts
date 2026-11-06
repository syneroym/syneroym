import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activate, currentLanguage } from "./i18n";
import { languagePicker } from "./picker";

let bar: HTMLElement;

beforeEach(async () => {
  localStorage.clear();
  await activate("en");
  bar = document.createElement("div");
  document.body.replaceChildren(bar);
});

afterEach(() => {
  vi.restoreAllMocks();
});

function choose(select: HTMLSelectElement, tag: string): void {
  select.value = tag;
  select.dispatchEvent(new Event("change"));
}

describe("languagePicker", () => {
  it("offers each language by its own name and selects the current one", () => {
    const select = languagePicker();
    bar.appendChild(select);
    const options = Array.from(select.options).map((o) => [o.value, o.textContent]);
    expect(options).toEqual([
      ["en", "English"],
      ["hi", "हिन्दी"],
    ]);
    expect(select.value).toBe("en");
    expect(select.getAttribute("aria-label")).toBe("Language");
  });

  it("saves the choice and reloads the page", () => {
    const reload = vi.fn();
    const select = languagePicker(reload);
    bar.appendChild(select);
    choose(select, "hi");
    expect(localStorage.getItem("roym-hub-language")).toBe("hi");
    expect(reload).toHaveBeenCalledOnce();
    expect(bar.querySelector(".language-note")).toBeNull();
  });

  it("does not reload, and says why, when the browser will not keep the choice", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const reload = vi.fn();
    const select = languagePicker(reload);
    bar.appendChild(select);
    choose(select, "hi");
    expect(reload).not.toHaveBeenCalled();
    expect(select.value).toBe(currentLanguage());
    expect(bar.querySelector(".language-note")?.textContent).toBe(
      "This browser does not let the Hub save the language, so it stays as it is.",
    );
    choose(select, "hi");
    expect(bar.querySelectorAll(".language-note")).toHaveLength(1);
  });
});
