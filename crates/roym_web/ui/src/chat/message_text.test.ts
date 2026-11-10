import { afterEach, describe, expect, it } from "vitest";
import { activate, t } from "../i18n/i18n";
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
