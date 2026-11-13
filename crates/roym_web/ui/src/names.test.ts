import { describe, expect, it } from "vitest";

import { scriptOf, sortByName } from "./names";

describe("scriptOf", () => {
  it("identifies scripts and non-letter cases correctly", () => {
    expect(scriptOf("")).toBe("Zzzz");
    expect(scriptOf("   ")).toBe("Zzzz");
    expect(scriptOf("Alice")).toBe("Latn");
    expect(scriptOf("राम")).toBe("Deva");
    expect(scriptOf("சென்னை")).toBe("Taml");
    expect(scriptOf("123")).toBe("Zzzz");
    expect(scriptOf("@#$")).toBe("Zzzz");
    expect(scriptOf("🚀🎉")).toBe("Zzzz");
    // Georgian letter ა is not in our 17 listed scripts -> Zyyy
    expect(scriptOf("ა")).toBe("Zyyy");
    // Letter after spaces
    expect(scriptOf("   Bob")).toBe("Latn");
    expect(scriptOf("   कमल")).toBe("Deva");
  });
});

describe("sortByName", () => {
  const items = [
    { id: "ta", name: "வாலா" },
    { id: "deva", name: "कमल" },
    { id: "latn", name: "Bob" },
    { id: "zyyy", name: "ა" },
    { id: "zzzz", name: "123" },
  ];

  it("sorts Latin first for en", () => {
    const sorted = sortByName(items, (i) => i.name, "en");
    expect(sorted.map((i) => i.id)).toEqual(["latn", "deva", "ta", "zyyy", "zzzz"]);
  });

  it("sorts Devanagari first for hi", () => {
    const sorted = sortByName(items, (i) => i.name, "hi");
    expect(sorted.map((i) => i.id)).toEqual(["deva", "latn", "ta", "zyyy", "zzzz"]);
  });

  it("sorts Tamil first for ta", () => {
    const sorted = sortByName(items, (i) => i.name, "ta");
    expect(sorted.map((i) => i.id)).toEqual(["ta", "latn", "deva", "zyyy", "zzzz"]);
  });

  it("uses numeric collation", () => {
    const list = ["a10", "a2", "a1"];
    const sorted = sortByName(list, (s) => s, "en");
    expect(sorted).toEqual(["a1", "a2", "a10"]);
  });

  it("is stable on equal names", () => {
    const list = [
      { id: 1, name: "Same" },
      { id: 2, name: "Same" },
      { id: 3, name: "Same" },
    ];
    const sorted = sortByName(list, (i) => i.name, "en");
    expect(sorted.map((i) => i.id)).toEqual([1, 2, 3]);
  });

  it("handles empty list and one item", () => {
    expect(sortByName([], (s: string) => s, "en")).toEqual([]);
    expect(sortByName(["single"], (s) => s, "en")).toEqual(["single"]);
  });

  it("defaults gracefully to Latin-first on unsupported lang", () => {
    const sorted = sortByName(items, (i) => i.name, "not-a-valid-tag!!!");
    expect(sorted.map((i) => i.id)).toEqual(["latn", "deva", "ta", "zyyy", "zzzz"]);
  });
});
