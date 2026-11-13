import { afterEach, describe, expect, it, vi } from "vitest";
import { activate, t } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { ENGINE, fakeEngine, readerReads, unavailableEngine } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { messageLang, messageText } from "./message_text";

describe("messageText and messageLang", () => {
  afterEach(async () => {
    await activate("en");
  });

  it("messageLang returns langAttr of first tag, or empty string", () => {
    expect(messageLang([])).toBe("");
    expect(messageLang(["ta-Latn"])).toBe("ta-Latn");
    expect(messageLang(["hi", "en"])).toBe("hi");
    expect(messageLang(["hi_IN"])).toBe("");
  });

  it("['hi', 'en']: body span has lang='hi' and text byte for byte; no note", () => {
    const text = "नमस्ते hello world 123";
    const nodes = messageText(text, ["hi", "en"]);
    expect(nodes).toHaveLength(1);
    const span = nodes[0] as HTMLElement;
    expect(span.tagName.toLowerCase()).toBe("span");
    expect(span.className).toBe("message-body");
    expect(span.getAttribute("lang")).toBe("hi");
    expect(span.textContent).toBe(text);
  });

  it("[]: lang='' and note 'Language not stated'; in Hindi, note has Devanagari letter", async () => {
    const nodesEn = messageText("some message", []);
    expect(nodesEn).toHaveLength(2);
    const bodyEn = nodesEn[0] as HTMLElement;
    expect(bodyEn.getAttribute("lang")).toBe("");
    const noteEn = nodesEn[1] as HTMLElement;
    expect(noteEn.className).toBe("message-language-note");
    expect(noteEn.textContent).toBe("Language not stated");

    await activate("hi");
    const nodesHi = messageText("कुछ संदेश", []);
    expect(nodesHi).toHaveLength(2);
    const noteHi = nodesHi[1] as HTMLElement;
    expect(noteHi.textContent).toBe(t("chat-text-not-stated"));
    expect(/[\u0900-\u097F]/.test(noteHi.textContent || "")).toBe(true);
  });

  it("['hi_IN'] (refused tag): lang='', no note", () => {
    const nodes = messageText("text", ["hi_IN"]);
    expect(nodes).toHaveLength(1);
    const span = nodes[0] as HTMLElement;
    expect(span.getAttribute("lang")).toBe("");
  });

  it("supports custom tag element such as div", () => {
    const nodes = messageText("hello", ["en"], "div");
    expect(nodes).toHaveLength(1);
    const div = nodes[0] as HTMLElement;
    expect(div.tagName.toLowerCase()).toBe("div");
    expect(div.className).toBe("message-body");
    expect(div.getAttribute("lang")).toBe("en");
  });
});

describe("messageText with a translation engine", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    setTargetsForTest([]);
  });

  function draw(body: string, languages: string[], translatable = true): HTMLElement {
    const host = document.createElement("div");
    host.append(...messageText(body, languages, "span", { translatable }));
    return host;
  }

  it("a node with no engine draws no translate button and makes no call", () => {
    readerReads([]);
    const calls = stubRpc({});
    expect(draw("वणक्कम", ["hi"]).querySelector(".message-translate")).toBeNull();
    expect(calls).toEqual([]);
  });

  it("translates on a click, labels the result, and leaves the body alone", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const host = draw("नमस्ते", ["hi"]);
    const button = host.querySelector<HTMLButtonElement>(".message-translate")!;
    expect(button.textContent).toBe("Translate into English");

    button.click();
    await vi.waitFor(() => expect(host.querySelector(".message-translation")).not.toBeNull());
    expect(seen).toEqual([{ texts: ["नमस्ते"], source_lang: "hi", target_lang: "en" }]);
    const translation = host.querySelector(".message-translation")!;
    expect([translation.textContent, translation.getAttribute("lang")]).toEqual(["T:नमस्ते", "en"]);
    expect(host.querySelector(".message-translation-note")!.textContent).toBe(
      `Machine translation into English by ${ENGINE}.`,
    );
    expect(host.querySelector(".message-body")!.textContent).toBe("नमस्ते");
    expect(host.querySelector(".message-translate")).toBeNull();
  });

  it("an untagged message sends no source language", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    stubRpc({ "translation.translate": fakeEngine(seen) });
    const host = draw("hello?", []);
    host.querySelector<HTMLButtonElement>(".message-translate")!.click();
    await vi.waitFor(() => expect(seen).toHaveLength(1));
    expect(seen[0]).toEqual({ texts: ["hello?"], target_lang: "en" });
  });

  it("keeps the body and says why when the engine is unavailable", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    stubRpc({ "translation.translate": unavailableEngine });
    const host = draw("नमस्ते", ["hi"]);
    host.querySelector<HTMLButtonElement>(".message-translate")!.click();
    await vi.waitFor(() => expect(host.querySelector(".message-translation-error")).not.toBeNull());
    expect(host.querySelector(".message-translation-error")!.textContent).toContain(
      "Could not translate",
    );
    expect(host.querySelector(".message-body")!.textContent).toBe("नमस्ते");
    expect(host.querySelector<HTMLButtonElement>(".message-translate")!.disabled).toBe(false);
  });

  it("a message already tagged in a language the reader reads gets no button", () => {
    readerReads([]);
    setTargetsForTest(["en", "hi"]);
    expect(draw("hello", ["en"]).querySelector(".message-translate")).toBeNull();
    // English is the second tag.
    expect(draw("नमस्ते hello", ["hi", "en"]).querySelector(".message-translate")).toBeNull();
  });

  it("a message that is not translatable gets no button", () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    expect(draw("नमस्ते", ["hi"], false).querySelector(".message-translate")).toBeNull();
  });
});
