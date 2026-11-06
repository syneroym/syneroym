import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  activate,
  activateWith,
  availableLanguages,
  currentLanguage,
  formatLocale,
  initLanguageWith,
  lookupText,
  messageId,
  pickLanguage,
  saveLanguage,
  t,
} from "./i18n";

beforeEach(async () => {
  localStorage.clear();
  await activate("en");
});

afterEach(async () => {
  vi.restoreAllMocks();
  await activate("en");
});

describe("pickLanguage", () => {
  const available = ["en", "hi"];

  it("prefers the stored choice when it is available", () => {
    expect(pickLanguage("hi", ["en-US"], available)).toBe("hi");
  });

  it("ignores a stored choice that is not available", () => {
    expect(pickLanguage("xx", [], available)).toBe("en");
    expect(pickLanguage("xx", ["hi-IN"], available)).toBe("hi");
  });

  it("matches the browser's languages in order", () => {
    expect(pickLanguage(null, ["en-US"], available)).toBe("en");
    expect(pickLanguage(null, ["hi-IN", "en"], available)).toBe("hi");
    expect(pickLanguage(null, ["ta", "hi"], available)).toBe("hi");
  });

  it("does not match a catalog written in another script", () => {
    expect(pickLanguage(null, ["hi-Latn"], available)).toBe("en");
  });

  it("skips a tag that does not parse", () => {
    expect(pickLanguage(null, ["x-private", "hi"], available)).toBe("hi");
    expect(pickLanguage(null, [], available)).toBe("en");
  });
});

describe("activate", () => {
  it("sets the page language and the format locale", async () => {
    await activate("hi");
    expect(document.documentElement.lang).toBe("hi");
    expect(currentLanguage()).toBe("hi");
    expect(formatLocale()).toBe("hi-IN-u-nu-latn");
  });

  it("keeps the region of a tag that has one", async () => {
    await activateWith("pt-BR", { "pt-BR": async () => "" });
    expect(formatLocale()).toBe("pt-BR-u-nu-latn");
  });

  it("adds India to a tag without a region", async () => {
    await activate("en");
    expect(formatLocale()).toBe("en-IN-u-nu-latn");
  });

  it("changes nothing when the catalog cannot load", async () => {
    const loaders = { hi: async () => Promise.reject(new Error("offline")) };
    await expect(activateWith("hi", loaders)).rejects.toThrow("offline");
    expect(document.documentElement.lang).toBe("en");
    expect(currentLanguage()).toBe("en");
  });

  it("falls back to English for a message the language lacks", async () => {
    await activateWith("xx", { xx: async () => "age-moments = xx moments" });
    expect(t("age-moments")).toBe("xx moments");
    expect(t("language-picker-label")).toBe("Language");
  });
});

describe("t", () => {
  it("returns the id and reports it when no catalog has it", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(t("no-such-message")).toBe("no-such-message");
    expect(error).toHaveBeenCalled();
  });

  it("does not throw for an id that is not a string", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const inherited = ({} as { toString: unknown }).toString as unknown as string;
    expect(() => t(inherited)).not.toThrow();
    expect(typeof t(inherited)).toBe("string");
  });

  it("fills variables without isolation marks", () => {
    expect(t("money-input-invalid", { input: "x", example: "12" })).toBe(
      '"x" is not an amount like 12 for this currency',
    );
  });

  it("is English with no setup in a fresh module", async () => {
    vi.resetModules();
    const fresh = await import("./i18n");
    expect(fresh.t("age-moments")).toBe("moments ago");
  });
});

describe("lookupText", () => {
  const table = new Map([["a", messageId("age-moments")]]);

  it("gives the text of a known key", () => {
    expect(lookupText(table, "a")).toBe("moments ago");
  });

  it("gives undefined for an unknown key, including inherited names", () => {
    for (const key of ["b", "constructor", "toString", "__proto__"]) {
      expect(lookupText(table, key)).toBeUndefined();
    }
  });
});

describe("the stored choice", () => {
  it("is picked up by initLanguageWith", async () => {
    expect(saveLanguage("hi")).toBe(true);
    await initLanguageWith({ hi: async () => "age-moments = अभी" }, ["en"]);
    expect(currentLanguage()).toBe("hi");
    expect(t("age-moments")).toBe("अभी");
  });

  it("is not lost when its catalog fails to load", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    saveLanguage("hi");
    const loaders = { hi: async () => Promise.reject(new Error("offline")) };
    await expect(initLanguageWith(loaders, [])).resolves.toBeUndefined();
    expect(t("age-moments")).toBe("moments ago");
    expect(document.documentElement.lang).toBe("en");
    expect(error).toHaveBeenCalled();
    expect(localStorage.getItem("roym-hub-language")).toBe("hi");
  });

  it("is reported as unsaved when storage throws", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(saveLanguage("hi")).toBe(false);
    await expect(initLanguageWith({}, ["en"])).resolves.toBeUndefined();
    expect(currentLanguage()).toBe("en");
  });
});

describe("availableLanguages", () => {
  it("lists English first, then the other catalogs sorted", () => {
    expect(availableLanguages({ ta: async () => "", hi: async () => "" })).toEqual(["en", "hi", "ta"]);
  });
});
