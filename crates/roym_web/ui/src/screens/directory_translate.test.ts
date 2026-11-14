import { afterEach, describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import { ENGINE, fakeEngine, readerReads, unavailableEngine } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { renderDirectory } from "./directory";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setTargetsForTest([]);
});

const FOUND = {
  run_id: "run-1",
  outcomes: [],
  hits: [],
  hits_truncated: false,
  refused: [],
  refused_truncated: false,
};

const HINDI_HIT = {
  listing_id: "lst_hi",
  record_id: "rec_hi",
  issuer: "did:key:zListingOwner",
  title: "नल वाला",
  summary: "घर पर सेवा",
  categories: [],
  conversation_address: "did:key:zConversation",
  status: "active",
  verified: true,
  revocation_status: "none",
  age_secs: 10,
  sources: [],
  versions_differ: false,
  lang: "hi",
  translations: [],
};

async function drawn(extra: Record<string, (p: Record<string, unknown>) => unknown> = {}) {
  const queries: Record<string, unknown>[] = [];
  const calls = stubRpc({
    "directory.sources": () => ({ sources: [] }),
    "directory.find": (params) => {
      queries.push(params);
      return FOUND;
    },
    ...extra,
  });
  const host = document.createElement("div");
  await renderDirectory(host);
  return { host, queries, calls };
}

const box = (host: HTMLElement) => host.querySelector<HTMLInputElement>(".search-text-input")!;

describe("the search box with a translation engine", () => {
  it("a node with no engine draws no translate controls and makes no call", async () => {
    readerReads([]);
    const { host, calls } = await drawn();
    expect(host.querySelector(".search-translate")).toBeNull();
    expect(host.querySelector(".search-translate-target")).toBeNull();
    expect(calls.some((c) => c.method === "translation.translate")).toBe(false);
  });

  it("translates the words in the box, and the next search sends them", async () => {
    readerReads([]);
    setTargetsForTest(["en", "hi"]);
    const seen: Record<string, unknown>[] = [];
    const { host, queries } = await drawn({ "translation.translate": fakeEngine(seen) });
    const select = host.querySelector<HTMLSelectElement>(".search-translate-target")!;
    expect(Array.from(select.options).map((o) => o.value)).toEqual(["en", "hi"]);
    expect(select.getAttribute("aria-label")).toBe("Language to search in");

    select.value = "hi";
    box(host).value = "plumber";
    host.querySelector<HTMLButtonElement>(".search-translate")!.click();
    await vi.waitFor(() => expect(box(host).value).toBe("T:plumber"));
    expect(seen).toEqual([{ texts: ["plumber"], target_lang: "hi" }]);
    expect(host.querySelector(".search-translate-note")!.textContent).toBe(
      `Machine translation into Hindi by ${ENGINE}.`,
    );

    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(queries).toHaveLength(1));
    expect((queries[0].query as Record<string, unknown>).text).toBe("T:plumber");
  });

  it("an empty box makes no call", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    const { host } = await drawn({ "translation.translate": fakeEngine(seen) });
    host.querySelector<HTMLButtonElement>(".search-translate")!.click();
    await Promise.resolve();
    expect(seen).toEqual([]);
  });

  it("keeps the words and says why when the engine is unavailable", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const { host } = await drawn({ "translation.translate": unavailableEngine });
    box(host).value = "नल वाला";
    host.querySelector<HTMLButtonElement>(".search-translate")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".search-translate-note")!.textContent).toBe("Translation is not available."),
    );
    expect(box(host).value).toBe("नल वाला");
  });
});

describe("a search hit with a translation engine", () => {
  async function withHit(extra: Record<string, (p: Record<string, unknown>) => unknown> = {}) {
    const found = { ...FOUND, hits: [HINDI_HIT] };
    const { host, calls } = await drawn({ "directory.find": () => found, ...extra });
    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(host.querySelectorAll(".search-hit")).toHaveLength(1));
    return { host, calls };
  }

  it("a node with no engine draws no translate action and makes no call", async () => {
    readerReads([]);
    const { host, calls } = await withHit();
    expect(host.querySelector(".record-text-translate")).toBeNull();
    expect(calls.some((c) => c.method === "translation.translate")).toBe(false);
  });

  it("sends the title and summary and shows the labelled result", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const seen: Record<string, unknown>[] = [];
    const { host } = await withHit({ "translation.translate": fakeEngine(seen) });
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(seen).toHaveLength(1));
    expect(seen[0]).toEqual({
      texts: ["नल वाला", "घर पर सेवा"],
      source_lang: "hi",
      target_lang: "en",
    });
    await vi.waitFor(() => expect(host.querySelector(".hit-title")!.textContent).toBe("T:नल वाला"));
    expect(host.querySelector(".hit-summary")!.textContent).toBe("T:घर पर सेवा");
    expect(host.querySelector(".record-text-note")!.textContent).toContain(ENGINE);
  });

  it("keeps the original text and says why when the engine is unavailable", async () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    const { host } = await withHit({ "translation.translate": unavailableEngine });
    host.querySelector<HTMLButtonElement>(".record-text-translate")!.click();
    await vi.waitFor(() => expect(host.querySelector(".record-text-error")).not.toBeNull());
    expect(host.querySelector(".hit-title")!.textContent).toBe("नल वाला");
  });
});
