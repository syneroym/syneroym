import { describe, expect, it } from "vitest";
import { text } from "../dom";
import { langAttr } from "../i18n/text_lang";
import {
  bestVersion,
  recordText,
  type TextVersion,
} from "./record_text";

describe("bestVersion", () => {
  it("original hi with translation en", () => {
    const versions: TextVersion<{ text: string }>[] = [
      { lang: "hi", fields: { text: "मूल" }, byAuthor: false },
      { lang: "en", fields: { text: "Original" }, byAuthor: true },
    ];

    expect(bestVersion(versions, ["en"])).toBe(1);
    expect(bestVersion(versions, ["hi"])).toBe(0);
    expect(bestVersion(versions, ["fr"])).toBe(0);
    expect(bestVersion(versions, ["fr", "en-GB"])).toBe(1);
  });

  it("original en with translations hi, hi-Latn", () => {
    const versions: TextVersion<{ text: string }>[] = [
      { lang: "en", fields: { text: "Title" }, byAuthor: false },
      { lang: "hi", fields: { text: "शीर्षक" }, byAuthor: true },
      { lang: "hi-Latn", fields: { text: "Sheershak" }, byAuthor: true },
    ];

    expect(bestVersion(versions, ["hi-Latn"])).toBe(2);
  });

  it("reader he with translation iw picks it", () => {
    const versions: TextVersion<{ text: string }>[] = [
      { lang: "en", fields: { text: "Title" }, byAuthor: false },
      { lang: "iw", fields: { text: "כותרת" }, byAuthor: true },
    ];

    expect(bestVersion(versions, ["he"])).toBe(1);
  });

  it("refused tag is never picked", () => {
    const versions: TextVersion<{ text: string }>[] = [
      { lang: "en", fields: { text: "Title" }, byAuthor: false },
      { lang: "hi_IN", fields: { text: "शीर्षक" }, byAuthor: true },
    ];

    expect(bestVersion(versions, ["hi"])).toBe(0);
    expect(bestVersion(versions, ["hi_IN"])).toBe(0);
  });
});

describe("recordText", () => {
  it("with one version: no note, no button, lang set", () => {
    const host = document.createElement("div");
    const versions: TextVersion<{ title: string }>[] = [
      { lang: "hi", fields: { title: "झाड़ी कटाई" }, byAuthor: false },
    ];

    recordText(host, versions, (v) => {
      const el = text("div", v.fields.title, "title");
      const attr = langAttr(v.lang);
      if (attr) el.setAttribute("lang", attr);
      return [el];
    });

    expect(host.querySelector(".record-text-note")).toBeNull();
    expect(host.querySelector(".record-text-switch")).toBeNull();
    const titleEl = host.querySelector(".title");
    expect(titleEl).not.toBeNull();
    expect(titleEl?.textContent).toBe("झाड़ी कटाई");
    expect(titleEl?.getAttribute("lang")).toBe("hi");
  });

  it("with multiple versions: displays best version, note and switch button", () => {
    const host = document.createElement("div");
    const versions: TextVersion<{ title: string }>[] = [
      { lang: "hi", fields: { title: "झाड़ी कटाई" }, byAuthor: false },
      { lang: "en", fields: { title: "Hedge trimming" }, byAuthor: true },
    ];

    recordText(host, versions, (v) => {
      const el = text("div", v.fields.title, "title");
      const attr = langAttr(v.lang);
      if (attr) el.setAttribute("lang", attr);
      return [el];
    });

    const note = host.querySelector(".record-text-note");
    const btn = host.querySelector(".record-text-switch") as HTMLButtonElement | null;
    expect(note).not.toBeNull();
    expect(btn).not.toBeNull();

    // Default reader in English: shows translation "Hedge trimming"
    const titleEl = host.querySelector(".title");
    expect(titleEl?.textContent).toBe("Hedge trimming");
    expect(titleEl?.getAttribute("lang")).toBe("en");
    expect(note?.textContent).toContain("Hindi");
    expect(btn?.textContent).toBe("Show the original");

    // Click switch button: toggles to original
    btn?.click();
    expect(host.querySelector(".title")?.textContent).toBe("झाड़ी कटाई");
    expect(host.querySelector(".title")?.getAttribute("lang")).toBe("hi");
    expect(note?.textContent).toContain("Original text");
    expect(btn?.textContent).toBe("Show the author's translation");

    // Click switch button again: toggles back
    btn?.click();
    expect(host.querySelector(".title")?.textContent).toBe("Hedge trimming");
    expect(host.querySelector(".title")?.getAttribute("lang")).toBe("en");
  });
});
