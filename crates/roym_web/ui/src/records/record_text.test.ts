import { afterEach, describe, expect, it, vi } from "vitest";
import { text } from "../dom";
import { activate } from "../i18n/i18n";
import { langAttr } from "../i18n/text_lang";
import { stubRpc } from "../testing/stub-rpc";
import { ENGINE, fakeEngine, readerReads, unavailableEngine } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import {
  bestVersion,
  recordText,
  recordValue,
  textFields,
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

  it("in Hindi, the note and the button have Devanagari letters", async () => {
    await activate("hi");
    try {
      const host = document.createElement("div");
      const versions: TextVersion<{ title: string }>[] = [
        { lang: "en", fields: { title: "Hedge trimming" }, byAuthor: false },
        { lang: "hi", fields: { title: "झाड़ी कटाई" }, byAuthor: true },
      ];

      recordText(host, versions, (v) => [text("div", v.fields.title, "title")]);

      const note = host.querySelector(".record-text-note");
      const btn = host.querySelector(".record-text-switch");
      expect(note).not.toBeNull();
      expect(btn).not.toBeNull();
      expect(note?.textContent).toMatch(/[ऀ-ॿ]/);
      expect(btn?.textContent).toMatch(/[ऀ-ॿ]/);
    } finally {
      await activate("en");
    }
  });
});

describe("recordValue", () => {
  it("with canonical tag sets lang attribute and text", () => {
    const span = recordValue("en", "Hello");
    expect(span.getAttribute("lang")).toBe("en");
    expect(span.textContent).toBe("Hello");
  });

  it("with unknown or refused tag gives an empty lang", () => {
    const span = recordValue("not_a_valid_bcp47_tag----", "Unset");
    expect(span.getAttribute("lang")).toBe("");
    expect(span.textContent).toBe("Unset");
  });

  it("with Node value appends child", () => {
    const a = document.createElement("a");
    a.href = "https://example.com";
    a.textContent = "Link";
    const span = recordValue("ta", a);
    expect(span.getAttribute("lang")).toBe("ta");
    expect(span.querySelector("a")).toBe(a);
  });
});

type Title = { title: string };

const TITLE_TEXTS = textFields<Title>(["title"]);

function drawTitle(v: TextVersion<Title>): Node[] {
  const el = text("div", v.fields.title, "title");
  el.setAttribute("lang", langAttr(v.lang));
  return [el];
}

function draw(versions: TextVersion<Title>[]): HTMLElement {
  const host = document.createElement("div");
  recordText(host, versions, drawTitle, TITLE_TEXTS);
  return host;
}

const HINDI_ONLY: TextVersion<Title>[] = [{ lang: "hi", fields: { title: "नल वाला" }, byAuthor: false }];
const HINDI_WITH_ENGLISH: TextVersion<Title>[] = [
  { lang: "hi", fields: { title: "नल वाला" }, byAuthor: false },
  { lang: "en", fields: { title: "Plumber" }, byAuthor: true },
];

function shownOf(host: HTMLElement): [string | undefined, string | undefined] {
  return [host.dataset.shownLang, host.dataset.shownVia];
}

afterEach(async () => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setTargetsForTest([]);
  await activate("en");
});

describe("recordText with a translation engine", () => {
  it("a node with no engine draws no translate action and makes no call", () => {
    readerReads([]);
    const calls = stubRpc({});
    const one = draw(HINDI_ONLY);
    const two = draw(HINDI_WITH_ENGLISH);
    expect(one.querySelector(".record-text-translate")).toBeNull();
    expect(two.querySelector(".record-text-translate")).toBeNull();
    expect(calls).toEqual([]);
  });

  it("a one-version record says what is on screen, with no note", () => {
    readerReads([]);
    const host = draw(HINDI_ONLY);
    expect(shownOf(host)).toEqual(["hi", "original"]);
    expect(host.querySelector(".record-text-meta")).toBeNull();
  });

  it("translates once, labels the result, and keeps the original one click away", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const host = draw(HINDI_ONLY);
    expect(shownOf(host)).toEqual(["hi", "original"]);
    const button = host.querySelector<HTMLButtonElement>(".record-text-translate")!;
    expect(button.textContent).toBe("Translate into English");

    button.click();
    await vi.waitFor(() => expect(host.querySelector(".record-text-translate")).toBeNull());
    expect(seen).toEqual([{ texts: ["नल वाला"], source_lang: "hi", target_lang: "en" }]);
    expect(host.querySelector(".title")!.textContent).toBe("T:नल वाला");
    expect(host.querySelector(".title")!.getAttribute("lang")).toBe("en");
    expect(host.querySelector(".record-text-note")!.textContent).toBe(
      `Machine translation into English by ${ENGINE}.`,
    );
    expect(shownOf(host)).toEqual(["en", "machine"]);
    expect(host.querySelector(".record-text-switch")!.textContent).toBe("Show the original");
  });

  it("says when the engine sent the text off the device", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    stubRpc({ "translation.translate": fakeEngine([], true) });
    const host = draw(HINDI_ONLY);
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(host.querySelector(".record-text-translate")).toBeNull());
    expect(host.querySelector(".record-text-note")!.textContent).toContain("sent off this device");
  });

  it("keeps the original and says why when the engine is unavailable", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    stubRpc({ "translation.translate": unavailableEngine });
    const host = draw(HINDI_ONLY);
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(host.querySelector(".record-text-error")).not.toBeNull());
    expect(host.querySelector(".record-text-error")!.textContent).toBe("Translation is not available.");
    expect(host.querySelector(".title")!.textContent).toBe("नल वाला");
    expect(shownOf(host)).toEqual(["hi", "original"]);
    expect(host.querySelector<HTMLButtonElement>(".record-text-translate")!.disabled).toBe(false);
  });

  it("walks original, author translation and machine version in order, saying what is shown", async () => {
    readerReads(["en", "ta"]);
    setTargetsForTest(["ta"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const host = draw(HINDI_WITH_ENGLISH);
    const switchBtn = () => host.querySelector<HTMLButtonElement>(".record-text-switch")!;
    const title = () => host.querySelector(".title")!.textContent;

    // The English author translation is the best version for this reader.
    expect(shownOf(host)).toEqual(["en", "author"]);
    switchBtn().click();
    expect([shownOf(host), title()]).toEqual([["hi", "original"], "नल वाला"]);
    switchBtn().click();
    expect([shownOf(host), title()]).toEqual([["en", "author"], "Plumber"]);

    // The machine version is made from the original, not from the English
    // translation the person was reading.
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(shownOf(host)).toEqual(["ta", "machine"]));
    expect(seen).toEqual([{ texts: ["नल वाला"], source_lang: "hi", target_lang: "ta" }]);
    expect(title()).toBe("T:नल वाला");
    expect(host.querySelector(".record-text-translate")).toBeNull();

    // The switch visits every version, so the author translation stays reachable.
    expect(switchBtn().textContent).toBe("Show the original");
    switchBtn().click();
    expect([shownOf(host), title()]).toEqual([["hi", "original"], "नल वाला"]);
    expect(switchBtn().textContent).toBe("Show the author's translation");
    switchBtn().click();
    expect([shownOf(host), title()]).toEqual([["en", "author"], "Plumber"]);
    expect(switchBtn().textContent).toBe("Translate into Tamil");
    switchBtn().click();
    expect([shownOf(host), title()]).toEqual([["ta", "machine"], "T:नल वाला"]);
  });

  it("draws a translation made earlier again, on screen if it was, with no second call", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const first = draw(HINDI_ONLY);
    first.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(shownOf(first)).toEqual(["en", "machine"]));

    // The thread is drawn again.
    const second = draw(HINDI_ONLY);
    expect(shownOf(second)).toEqual(["en", "machine"]);
    expect(second.querySelector(".title")!.textContent).toBe("T:नल वाला");
    expect(second.querySelector(".record-text-translate")).toBeNull();

    // The person goes back to the original and the thread is drawn again:
    // the original stays on screen, and the machine version is one click away.
    second.querySelector<HTMLButtonElement>(".record-text-switch")!.click();
    const third = draw(HINDI_ONLY);
    expect(shownOf(third)).toEqual(["hi", "original"]);
    third.querySelector<HTMLButtonElement>(".record-text-switch")!.click();
    expect(shownOf(third)).toEqual(["en", "machine"]);
    expect(seen).toHaveLength(1);
  });

  it("does not reuse a translation for a different text", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const first = draw(HINDI_ONLY);
    first.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(shownOf(first)).toEqual(["en", "machine"]));

    const other = draw([{ lang: "hi", fields: { title: "बिजली वाला" }, byAuthor: false }]);
    expect(shownOf(other)).toEqual(["hi", "original"]);
    expect(other.querySelector(".record-text-translate")).not.toBeNull();
  });

  it("draws no second translate action after one translation, also for another language", async () => {
    readerReads(["ta", "te"]);
    setTargetsForTest(["ta", "te"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const host = draw(HINDI_ONLY);
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(shownOf(host)).toEqual(["ta", "machine"]));
    expect(host.querySelector(".record-text-translate")).toBeNull();
    host.querySelector<HTMLButtonElement>(".record-text-switch")!.click();
    host.querySelector<HTMLButtonElement>(".record-text-switch")!.click();
    expect(shownOf(host)).toEqual(["ta", "machine"]);
    expect(seen).toHaveLength(1);
  });

  it("offers no translation into a language the record already has", async () => {
    await activate("hi");
    readerReads(["en"]);
    setTargetsForTest(["hi", "en"]);
    const calls = stubRpc({});
    const host = draw(HINDI_WITH_ENGLISH);
    expect(host.querySelector(".record-text-translate")).toBeNull();
    expect(calls).toEqual([]);
  });
});

describe("textFields", () => {
  const fields = textFields<{ a: string; link?: string; b?: string }>(["a", "link", "b"]);

  it("leaves a link out of what is sent and keeps it in what comes back", () => {
    const original = { a: "one", link: "https://example.org/x", b: "two" };
    expect(fields.get(original)).toEqual(["one", "two"]);
    expect(fields.set(original, ["T:one", "T:two"])).toEqual({
      a: "T:one",
      link: "https://example.org/x",
      b: "T:two",
    });
  });

  it("sends an absent field as an empty text", () => {
    expect(fields.get({ a: "one" })).toEqual(["one", "", ""]);
  });
});
