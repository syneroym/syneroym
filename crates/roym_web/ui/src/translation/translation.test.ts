import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { RpcError } from "../rpc";
import { ENGINE, readerReads } from "../testing/translation";
import { stubRpc } from "../testing/stub-rpc";
import {
  keepShown,
  keptTranslation,
  loadTargets,
  machineNote,
  searchTargets,
  setTargetsForTest,
  startTranslation,
  targetFor,
  translateTexts,
  translationFailure,
} from "./translation";

afterEach(async () => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setTargetsForTest([]);
  await activate("en");
});

describe("loadTargets", () => {
  it("keeps the list the node answers", async () => {
    stubRpc({ "translation.targets": () => ({ targets: ["en", "hi"] }) });
    await loadTargets();
    expect(searchTargets()).toEqual(["en", "hi"]);
  });

  it("an older node that does not know the method leaves no targets", async () => {
    setTargetsForTest(["en"]);
    stubRpc({ "translation.targets": () => new RpcError(-32601, "not found", "Other") });
    await loadTargets();
    expect(searchTargets()).toEqual([]);
  });

  it("a network failure leaves no targets", async () => {
    setTargetsForTest(["en"]);
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new Error("offline"))));
    await loadTargets();
    expect(searchTargets()).toEqual([]);
  });

  it("a node that does not answer in time leaves no targets", async () => {
    vi.useFakeTimers();
    setTargetsForTest(["en"]);
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));
    const loading = loadTargets();
    await vi.advanceTimersByTimeAsync(3000);
    await loading;
    expect(searchTargets()).toEqual([]);
  });
});

describe("startTranslation", () => {
  it("asks the node once for a signed-in person", async () => {
    const calls = stubRpc({ "translation.targets": () => ({ targets: ["en"] }) });
    await startTranslation(true);
    expect(calls.map((c) => c.method)).toEqual(["translation.targets"]);
  });

  it("makes no call for an anonymous page", async () => {
    const calls = stubRpc({});
    await startTranslation(false);
    expect(calls).toEqual([]);
  });
});

describe("targetFor", () => {
  it("skips the languages the text is already in", () => {
    readerReads([]);
    setTargetsForTest(["en", "hi"]);
    expect(targetFor(["hi"])).toBe("en");
    expect(targetFor(["en"])).toBeNull();
    expect(targetFor([])).toBe("en");
  });

  it("returns the node's own entry, not the reader's tag", () => {
    setTargetsForTest(["en"]);
    readerReads(["en-US"]);
    expect(targetFor(["hi"])).toBe("en");
    readerReads(["en-us"]);
    expect(targetFor(["hi"])).toBe("en");
  });

  it("gives nothing when every language the reader reads is already there", async () => {
    await activate("hi");
    readerReads(["en"]);
    setTargetsForTest(["hi", "en"]);
    expect(targetFor(["hi", "en"])).toBeNull();
  });

  it("matches he with iw", () => {
    readerReads(["he"]);
    setTargetsForTest(["iw"]);
    expect(targetFor(["en"])).toBe("iw");
  });

  it("a target called constructor is not an inherited name", () => {
    readerReads([]);
    setTargetsForTest(["constructor", "en"]);
    expect(targetFor(["hi"])).toBe("en");
  });

  it("gives nothing when the node has no engine", () => {
    setTargetsForTest([]);
    expect(targetFor([])).toBeNull();
  });
});

describe("translateTexts", () => {
  it("sends the texts, the source and the target", async () => {
    const calls = stubRpc({
      "translation.translate": () => ({
        items: [{ text: "a", source_lang: "hi", engine: { name: ENGINE, remote: false } }],
      }),
    });
    const out = await translateTexts(["x"], "hi", "en");
    expect(calls[0].params).toEqual({ texts: ["x"], source_lang: "hi", target_lang: "en" });
    expect(out).toEqual({ texts: ["a"], sourceLang: "hi", engine: { name: ENGINE, remote: false } });
  });

  it("omits an unknown source", async () => {
    const calls = stubRpc({
      "translation.translate": () => ({
        items: [{ text: "a", source_lang: "hi", engine: { name: ENGINE, remote: false } }],
      }),
    });
    await translateTexts(["x"], null, "en");
    expect(calls[0].params).toEqual({ texts: ["x"], target_lang: "en" });
  });

  it("throws the node's refusal", async () => {
    stubRpc({ "translation.translate": () => new RpcError(-32020, "no", "Other") });
    await expect(translateTexts(["x"], null, "en")).rejects.toBeInstanceOf(RpcError);
  });
});

describe("machineNote", () => {
  it("names the language and the engine, and says when the text left the device", () => {
    expect(machineNote("hi", { name: ENGINE, remote: false })).toBe(
      `Machine translation into Hindi by ${ENGINE}.`,
    );
    expect(machineNote("hi", { name: ENGINE, remote: true })).toBe(
      `Machine translation into Hindi by ${ENGINE}, sent off this device.`,
    );
  });
});

describe("translationFailure", () => {
  const refusal = (data: Record<string, unknown>, message = "service text") =>
    new RpcError(-32020, message, "Other", data);

  it("tells each kind of refusal in its own words", () => {
    expect(translationFailure(refusal({ kind: "unavailable" }))).toBe("Translation is not available.");
    expect(translationFailure(refusal({ kind: "unsupported-pair", source: "hi", target: "ta" }))).toBe(
      "The translation service cannot translate from Hindi to Tamil.",
    );
    expect(translationFailure(refusal({ kind: "too-long", limit: 100 }))).toBe(
      "The text is too long for the translation service.",
    );
    expect(translationFailure(refusal({ kind: "engine-failed" }, "engine said no"))).toBe(
      "The translation service failed. engine said no",
    );
  });

  it("does not nest a refusal inside another sentence", () => {
    for (const kind of ["unavailable", "unsupported-pair", "too-long", "engine-failed"]) {
      expect(translationFailure(refusal({ kind }))).not.toContain("Could not translate");
    }
  });

  it("shows a refusal of a kind it does not know as a service refusal", () => {
    expect(translationFailure(refusal({ kind: "new-kind" }, "why"))).toBe(
      "The translation service refused. why",
    );
    expect(translationFailure(new RpcError(-32020, "why", "Other"))).toBe(
      "The translation service refused. why",
    );
  });

  it("puts any other failure inside Could not translate", () => {
    expect(translationFailure(new RpcError(0, "offline", "Other"))).toMatch(/^Could not translate\. /);
    expect(translationFailure(new Error("boom"))).toMatch(/^Could not translate\. /);
  });
});

describe("kept translations", () => {
  const reply = () => ({
    items: [{ text: "a", source_lang: "hi", engine: { name: ENGINE, remote: false } }],
  });

  it("keeps a result for the same texts, source and target, and nothing for a refusal", async () => {
    stubRpc({ "translation.translate": reply });
    expect(keptTranslation(["x"], "hi", "en")).toBeUndefined();
    await translateTexts(["x"], "hi", "en");
    expect(keptTranslation(["x"], "hi", "en")?.out.texts).toEqual(["a"]);
    expect(keptTranslation(["x"], "hi", "ta")).toBeUndefined();
    expect(keptTranslation(["x"], null, "en")).toBeUndefined();
    expect(keptTranslation(["y"], "hi", "en")).toBeUndefined();

    stubRpc({ "translation.translate": () => new RpcError(-32020, "no", "Other") });
    await expect(translateTexts(["z"], "hi", "en")).rejects.toBeInstanceOf(RpcError);
    expect(keptTranslation(["z"], "hi", "en")).toBeUndefined();
  });

  it("remembers whether the translation was on screen", async () => {
    stubRpc({ "translation.translate": reply });
    await translateTexts(["x"], "hi", "en");
    expect(keptTranslation(["x"], "hi", "en")?.shown).toBe(false);
    keepShown(["x"], "hi", "en", true);
    expect(keptTranslation(["x"], "hi", "en")?.shown).toBe(true);
  });

  it("forgets the oldest results beyond its limit", async () => {
    stubRpc({ "translation.translate": reply });
    for (let i = 0; i < 201; i++) await translateTexts([`t${i}`], null, "en");
    expect(keptTranslation(["t0"], null, "en")).toBeUndefined();
    expect(keptTranslation(["t1"], null, "en")).toBeDefined();
    expect(keptTranslation(["t200"], null, "en")).toBeDefined();
  });
});
