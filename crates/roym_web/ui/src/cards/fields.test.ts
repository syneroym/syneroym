import { afterEach, describe, expect, it } from "vitest";
import { activate } from "../i18n/i18n";
import { formatDateTime, formatDuration, formatNumber } from "../i18n/format";
import { formatMinor } from "../money";
import type { SchemaDef } from "../extensions/registry";
import { fieldView } from "./fields";

const EXT = { id: "demo", version: 1 };

const SCHEMA: SchemaDef = {
  schema: "demo/form@1",
  fields: [
    { name: "note", type: "text", max_len: 50 },
    { name: "story", type: "long-text", max_len: 500 },
    { name: "ref", type: "code", max_len: 20 },
    { name: "count", type: "number" },
    { name: "price", type: "money" },
    { name: "at", type: "date-time" },
    { name: "length", type: "duration" },
    { name: "kind", type: "choice", choices: ["big", "small"] },
  ],
};

const LABELS = new Map([
  ["note", "ext-scheduling-slot-listing-id"],
  ["kind:big", "ext-scheduling-slot-slot-id"],
]);

function lines(el: HTMLElement): string[] {
  return [...el.querySelectorAll("p")].map((p) => p.textContent ?? "");
}

afterEach(async () => {
  await activate("en");
});

describe("fieldView of a known schema", () => {
  const data = {
    note: "Bring a hat",
    story: "line one\nline two",
    ref: "lst_123",
    count: 1234567,
    price: 250000,
    at: 1_800_000_000,
    length: 5400,
    kind: "big",
  };
  const view = () =>
    fieldView({ ext: EXT, schema: SCHEMA, data, currency: "INR", lang: "hi", labelIds: LABELS });

  it("draws one line per field, in manifest order, with a field the catalog lacks under its own name", () => {
    expect(lines(view())).toEqual([
      "Listing code: Bring a hat",
      "story line one\nline two",
      "ref lst_123",
      `count ${formatNumber(1234567)}`,
      `price ${formatMinor(250000, "INR")}`,
      `at ${formatDateTime(1_800_000_000)}`,
      `length ${formatDuration(5400)}`,
      "kind Slot code:",
    ]);
  });

  it("draws a duration as hours and minutes", () => {
    expect(formatDuration(5400)).toBe("1 hour 30 minutes");
    expect(formatDuration(3600)).toBe("1 hour");
    expect(formatDuration(600)).toBe("10 minutes");
    expect(formatDuration(0)).toBe("0 minutes");
  });

  it("gives text the record's language and a code none", () => {
    const el = view();
    expect(el.querySelector(".field-view-text span[lang]")?.getAttribute("lang")).toBe("hi");
    expect(el.querySelector(".field-view-long-text span[lang]")?.getAttribute("lang")).toBe("hi");
    expect(el.querySelector(".field-view-code span[lang]")).toBeNull();
  });

  it("leaves out a field that is not in the data", () => {
    const el = fieldView({ ext: EXT, schema: SCHEMA, data: { ref: "x" }, labelIds: LABELS });
    expect(lines(el)).toEqual(["ref x"]);
  });

  it("shows a value of the wrong type as it came, and money without a currency as a number", () => {
    const el = fieldView({
      ext: EXT,
      schema: SCHEMA,
      data: { count: "many", at: "soon", price: 5 },
      labelIds: LABELS,
    });
    expect(lines(el)).toEqual(["count many", `price ${formatNumber(5)}`, "at soon"]);
  });

  it("is not an unknown extension: no not-checked line", () => {
    expect(view().querySelector(".field-view-not-checked")).toBeNull();
  });
});

describe("fieldView of an unknown extension", () => {
  const unknown = {
    ext: { id: "future", version: 3 },
    data: { zeta: "last", alpha: { deep: [1] } },
    labelIds: new Map<string, string>(),
  };

  it("draws every key and value as it came, then says it was not checked", () => {
    const el = fieldView(unknown);
    expect(lines(el)).toEqual([
      "zeta: last",
      'alpha: {"deep":[1]}',
      "Not checked on this device",
    ]);
  });

  it("marks the keys and values as text in no known language", () => {
    const el = fieldView({ ...unknown, lang: "hi" });
    const spans = [...el.querySelectorAll(".field-view-data span")];
    expect(spans.length).toBeGreaterThan(0);
    for (const span of spans) expect(span.getAttribute("lang")).toBe("");
  });

  it("says it was not checked even when the data is empty", () => {
    expect(lines(fieldView({ ...unknown, data: {} }))).toEqual(["Not checked on this device"]);
  });
});

describe("fieldView extra lines", () => {
  const extra = [{ label: "Bring", value: "old reports" }];

  it("draws each extra line with the record's language on label and value", () => {
    const el = fieldView({ ext: EXT, schema: SCHEMA, data: {}, extra, lang: "hi", labelIds: LABELS });
    expect(lines(el)).toEqual(["Bring: old reports"]);
    const langs = [...el.querySelectorAll(".field-view-extra span")].map((s) => s.getAttribute("lang"));
    expect(langs).toEqual(["hi", "hi"]);
  });

  it("draws them after an unknown extension's data as well", () => {
    const el = fieldView({ ext: EXT, data: { a: 1 }, extra, labelIds: new Map() });
    expect(lines(el)).toEqual(["a: 1", "Not checked on this device", "Bring: old reports"]);
  });
});
