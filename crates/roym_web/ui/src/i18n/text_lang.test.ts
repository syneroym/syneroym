import { describe, expect, it } from "vitest";
import { activate, availableLanguages } from "./i18n";
import {
  RECORD_LANGUAGES,
  langAttr,
  languageName,
  sameLanguage,
  textLangSelect,
} from "./text_lang";

describe("sameLanguage", () => {
  it("matches same language with region", () => {
    expect(sameLanguage("hi", "hi-IN")).toBe(true);
  });

  it("does not match different scripts", () => {
    expect(sameLanguage("hi", "hi-Latn")).toBe(false);
  });

  it("matches legacy and modern language codes", () => {
    expect(sameLanguage("iw", "he")).toBe(true);
  });

  it("matches nothing on malformed tags", () => {
    expect(sameLanguage("hi_IN", "hi")).toBe(false);
    expect(sameLanguage("hi", "hi_IN")).toBe(false);
    expect(sameLanguage("", "hi")).toBe(false);
    expect(sameLanguage("hi", "")).toBe(false);
  });
});

describe("languageName", () => {
  it("returns English name in English", () => {
    expect(languageName("ta", "en")).toBe("Tamil");
  });

  it("returns name in active Hindi UI with Devanagari script", async () => {
    await activate("hi");
    const name = languageName("ta");
    expect(name).toMatch(/[\u0900-\u097F]/);
    await activate("en");
  });

  it("falls back to tag on invalid tag", () => {
    expect(languageName("hi_IN")).toBe("hi_IN");
  });
});

describe("langAttr", () => {
  it("returns tag for well-formed tag", () => {
    expect(langAttr("hi-Latn")).toBe("hi-Latn");
  });

  it("returns empty string for invalid tag", () => {
    expect(langAttr("hi_IN")).toBe("");
  });
});

describe("textLangSelect", () => {
  it("contains RECORD_LANGUAGES in order and sets current", () => {
    const sel = textLangSelect("test-class", "en");
    expect(sel.className).toBe("test-class");
    expect(sel.options.length).toBe(RECORD_LANGUAGES.length);
    for (let i = 0; i < RECORD_LANGUAGES.length; i++) {
      expect(sel.options[i].value).toBe(RECORD_LANGUAGES[i]);
      expect(sel.options[i].textContent).toBe(languageName(RECORD_LANGUAGES[i]));
    }
    expect(sel.value).toBe("en");
  });

  it("adds and selects current tag when not in RECORD_LANGUAGES", () => {
    const sel = textLangSelect("test-class", "sa");
    expect(sel.options.length).toBe(RECORD_LANGUAGES.length + 1);
    for (let i = 0; i < RECORD_LANGUAGES.length; i++) {
      expect(sel.options[i].value).toBe(RECORD_LANGUAGES[i]);
    }
    expect(sel.options[RECORD_LANGUAGES.length].value).toBe("sa");
    expect(sel.value).toBe("sa");
  });
});

describe("canonical syntax form", () => {
  it("every entry of RECORD_LANGUAGES equals its Intl.Locale string", () => {
    for (const tag of RECORD_LANGUAGES) {
      expect(new Intl.Locale(tag).toString()).toBe(tag);
    }
  });

  it("every catalog tag from availableLanguages equals its Intl.Locale string", () => {
    for (const tag of availableLanguages()) {
      expect(new Intl.Locale(tag).toString()).toBe(tag);
    }
  });
});
