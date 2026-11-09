import { afterEach, describe, expect, it } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith } from "../i18n/i18n";
import { leftoverLatin } from "../testing/latin-words";
import {
  buildSetListingParams,
  currencyMinorExponent,
  ListingInputError,
  MoneyInputError,
  slotFromLocalDatetimes,
  toMicroDegrees,
  toMinorUnits,
  toWholeCount,
  type ListingForm,
} from "./editor";

function emptyForm(): ListingForm {
  return {
    slug: "",
    title: "",
    summary: "",
    categories: [],
    conversation_address: "",
    status: "active",
    booking: {
      enabled: false,
      value: {
        mode: "enquiry",
        lead_time_secs: "0",
        cancellation_window_secs: "0",
        max_per_booking: "1",
      },
    },
    payment: {
      enabled: false,
      value: {
        currency: "eur",
        model: "per-hour",
        amount: "",
        tax_included: false,
        fees: "",
        methods: [],
        payee: "",
      },
    },
    product: {
      enabled: false,
      value: { unit: "item", pack_size: "1", condition: "new", sku: "" },
    },
    service: {
      enabled: false,
      value: { duration_secs: "3600", includes: "", excludes: "", prerequisites: "" },
    },
    location: {
      enabled: false,
      value: { where: "remote", address_disclosure: "on-agreement", area_label: "" },
    },
    relationship: {
      enabled: false,
      value: { open_to: "anyone", member_of: "" },
    },
    service_record: {
      enabled: false,
      value: { issues_fulfilment_receipt: false, warranty_secs: "0", retention_secs: "0" },
    },
  };
}

/// Every number anywhere in `value` is an integer -- the property the host
/// enforces before it signs, checked here on what the editor emits.
function everyNumberIsAnInteger(value: unknown): boolean {
  if (typeof value === "number") return Number.isInteger(value);
  if (Array.isArray(value)) return value.every(everyNumberIsAnInteger);
  if (value && typeof value === "object") {
    return Object.values(value).every(everyNumberIsAnInteger);
  }
  return true;
}

describe("toMinorUnits", () => {
  it("maps a decimal price to integer minor units", () => {
    expect(toMinorUnits("35.50")).toBe(3550);
    expect(toMinorUnits("35.5")).toBe(3550);
    expect(toMinorUnits("35")).toBe(3500);
    expect(toMinorUnits("0.09")).toBe(9);
  });

  it("treats an empty string as absent", () => {
    expect(toMinorUnits("")).toBeUndefined();
    expect(toMinorUnits("   ")).toBeUndefined();
  });

  it("rejects more than two decimal places and non-numeric input", () => {
    expect(() => toMinorUnits("35.555")).toThrow(MoneyInputError);
    expect(() => toMinorUnits("abc")).toThrow(MoneyInputError);
    expect(() => toMinorUnits("-1")).toThrow(MoneyInputError);
  });

  it("scales by the currency's own minor-unit exponent", () => {
    expect(toMinorUnits("1200", 0)).toBe(1200);
    expect(toMinorUnits("35.5", 3)).toBe(35500);
    expect(toMinorUnits("2.5", 3)).toBe(2500);
    // A JPY price with a fractional part, or a 3-place currency with four,
    // is rejected rather than truncated.
    expect(() => toMinorUnits("1200.5", 0)).toThrow(MoneyInputError);
    expect(() => toMinorUnits("2.5555", 3)).toThrow(MoneyInputError);
  });
});

describe("currencyMinorExponent", () => {
  it("knows the exponent-0 and exponent-3 currencies and defaults the rest to two", () => {
    expect(currencyMinorExponent("EUR")).toBe(2);
    expect(currencyMinorExponent("usd")).toBe(2);
    expect(currencyMinorExponent("JPY")).toBe(0);
    expect(currencyMinorExponent(" krw ")).toBe(0);
    expect(currencyMinorExponent("KWD")).toBe(3);
    expect(currencyMinorExponent("BHD")).toBe(3);
  });
});

describe("toMicroDegrees", () => {
  it("maps degrees to integer micro-degrees", () => {
    expect(toMicroDegrees("48.8566")).toBe(48856600);
    expect(toMicroDegrees("-2.3522")).toBe(-2352200);
    expect(toMicroDegrees("48")).toBe(48000000);
  });

  it("rejects a coordinate with too much precision", () => {
    expect(() => toMicroDegrees("48.85660123")).toThrow(ListingInputError);
  });
});

describe("toWholeCount", () => {
  it("accepts a whole number and rejects a decimal", () => {
    expect(toWholeCount("3600", "x")).toBe(3600);
    expect(toWholeCount("", "x")).toBeUndefined();
    expect(() => toWholeCount("3.5", "x")).toThrow(ListingInputError);
  });
});

describe("buildSetListingParams", () => {
  it("omits blocks whose checkbox is off", () => {
    const f = emptyForm();
    f.title = "Hedge trimming";
    const params = buildSetListingParams(f);
    expect(params).toEqual({
      title: "Hedge trimming",
      summary: "",
      lang: "en",
      categories: [],
      status: "active",
    });
  });

  it("emits an integer amount_minor for a decimal price and never a decimal anywhere", () => {
    const f = emptyForm();
    f.title = "Hedge trimming";
    f.categories = ["gardening", "outdoor"];
    f.payment.enabled = true;
    f.payment.value.amount = "35.50";
    f.payment.value.fees = "2.5";
    f.payment.value.methods = ["cash", "card"];
    f.payment.value.payee = "A. Gardener";
    f.product.enabled = true;
    f.product.value.unit = "piece";
    f.booking.enabled = true;
    f.booking.value.mode = "slots";
    f.booking.value.lead_time_secs = "3600";
    f.service.enabled = true;
    f.service.value.duration_secs = "1800";

    const params = buildSetListingParams(f);
    const payment = params.payment as Record<string, unknown>;
    const product = params.product as Record<string, unknown>;
    expect(payment.amount_minor).toBe(3550);
    expect(payment.fees_minor).toBe(250);
    expect(payment.currency).toBe("EUR");
    expect(payment.methods).toEqual(["cash", "card"]);
    expect(product.unit).toBe("piece");
    expect(params.categories).toEqual(["gardening", "outdoor"]);

    expect(everyNumberIsAnInteger(params)).toBe(true);
    // Belt and braces: the serialized form has no `<digit>.<digit>` run.
    expect(JSON.stringify(params)).not.toMatch(/\d\.\d/);
  });

  it("refuses a non-quote-only payment with no amount", () => {
    const f = emptyForm();
    f.title = "x";
    f.payment.enabled = true;
    f.payment.value.model = "fixed";
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
  });

  it("refuses a payment with a malformed amount and throws ListingInputError", () => {
    const f = emptyForm();
    f.title = "x";
    f.payment.enabled = true;
    f.payment.value.currency = "USD";
    f.payment.value.model = "fixed";
    f.payment.value.amount = "12.345";
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
    expect(() => buildSetListingParams(f)).toThrow(
      '"12.345" is not an amount like 12 or 12.50 for this currency',
    );
  });

  it("refuses a members-only relationship with no group DID", () => {
    const f = emptyForm();
    f.title = "x";
    f.relationship.enabled = true;
    f.relationship.value.open_to = "members";
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
  });

  it("sends lang and translations trimmed in row order", () => {
    const f = emptyForm();
    f.title = "Title";
    f.lang = "en";
    f.translations = [
      { lang: "hi", title: "  शीर्षक  ", summary: "  सार  " },
      { lang: "ta", title: "தலைப்பு", summary: "   " },
    ];
    const params = buildSetListingParams(f);
    expect(params.lang).toBe("en");
    expect(params.translations).toEqual([
      { lang: "hi", title: "शीर्षक", summary: "सार" },
      { lang: "ta", title: "தலைப்பு", summary: "" },
    ]);
  });

  it("refuses empty translation title", () => {
    const f = emptyForm();
    f.title = "Title";
    f.translations = [{ lang: "hi", title: "   ", summary: "" }];
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
    expect(() => buildSetListingParams(f)).toThrow("a translation title is required");
  });

  it("refuses translation in listing own language", () => {
    const f = emptyForm();
    f.title = "Title";
    f.lang = "en";
    f.translations = [{ lang: "en", title: "Tr", summary: "" }];
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
    expect(() => buildSetListingParams(f)).toThrow(
      "a translation cannot use the listing's own language",
    );
  });

  it("refuses duplicate translation language", () => {
    const f = emptyForm();
    f.title = "Title";
    f.translations = [
      { lang: "hi", title: "Tr 1", summary: "" },
      { lang: "hi", title: "Tr 2", summary: "" },
    ];
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
    expect(() => buildSetListingParams(f)).toThrow(
      "two translations cannot use the same language",
    );
  });
});

describe("slotFromLocalDatetimes", () => {
  it("produces integer second bounds", () => {
    const slot = slotFromLocalDatetimes("2026-09-10T09:00", "2026-09-10T10:00", "3");
    expect(Number.isInteger(slot.start_secs)).toBe(true);
    expect(Number.isInteger(slot.end_secs)).toBe(true);
    expect(slot.end_secs).toBeGreaterThan(slot.start_secs);
    expect(slot.capacity).toBe(3);
  });

  it("rejects an end at or before the start", () => {
    expect(() => slotFromLocalDatetimes("2026-09-10T10:00", "2026-09-10T10:00", "1")).toThrow(
      ListingInputError,
    );
  });
});

describe("every editor error, in Hindi", () => {
  afterEach(async () => {
    await activate("en");
  });

  type Block = "booking" | "payment" | "product" | "service" | "location" | "relationship" | "service_record";

  /// A form with one block on and one field changed.
  function withBlock(block: Block, change: Record<string, string>): () => unknown {
    return () => {
      const form = emptyForm();
      form.title = "Zq-title";
      form[block].enabled = true;
      Object.assign(form[block].value, change);
      return buildSetListingParams(form);
    };
  }

  /// The 11 places the editor throws, and the 8 names that go into a
  /// "whole number" message. Seven of those names also have an "is
  /// required" message; a slot capacity has the slot message instead.
  const THROWS: Array<[string, () => unknown]> = [
    ["coordinate", () => toMicroDegrees("Zq-north")],
    ["whole number: lead time", withBlock("booking", { lead_time_secs: "1.5" })],
    ["whole number: cancellation window", withBlock("booking", { cancellation_window_secs: "-1" })],
    ["whole number: max per booking", withBlock("booking", { max_per_booking: "x" })],
    ["whole number: pack size", withBlock("product", { pack_size: "1.5" })],
    ["whole number: duration", withBlock("service", { duration_secs: "1.5" })],
    ["whole number: warranty", withBlock("service_record", { warranty_secs: "1.5" })],
    ["whole number: retention", withBlock("service_record", { retention_secs: "1.5" })],
    ["whole number: capacity", () => slotFromLocalDatetimes("2030-01-01T10:00", "2030-01-01T11:00", "1.5")],
    ["required: lead time", withBlock("booking", { lead_time_secs: "" })],
    ["required: cancellation window", withBlock("booking", { cancellation_window_secs: "" })],
    ["required: max per booking", withBlock("booking", { max_per_booking: "" })],
    ["required: pack size", withBlock("product", { pack_size: "" })],
    ["required: duration", withBlock("service", { duration_secs: "" })],
    ["required: warranty", withBlock("service_record", { warranty_secs: "" })],
    ["required: retention", withBlock("service_record", { retention_secs: "" })],
    ["unknown currency", withBlock("payment", { currency: "zzz", amount: "1" })],
    ["amount required", withBlock("payment", { amount: "" })],
    ["quote-only with an amount", withBlock("payment", { model: "quote-only", amount: "5" })],
    ["members need a group", withBlock("relationship", { open_to: "members", member_of: "" })],
    ["title required", () => buildSetListingParams(emptyForm())],
    [
      "translation title required",
      () => {
        const f = emptyForm();
        f.title = "T";
        f.translations = [{ lang: "hi", title: "   ", summary: "" }];
        return buildSetListingParams(f);
      },
    ],
    [
      "translation same language",
      () => {
        const f = emptyForm();
        f.title = "T";
        f.lang = "en";
        f.translations = [{ lang: "en", title: "Tr", summary: "" }];
        return buildSetListingParams(f);
      },
    ],
    [
      "translation duplicate",
      () => {
        const f = emptyForm();
        f.title = "T";
        f.translations = [
          { lang: "hi", title: "Tr 1", summary: "" },
          { lang: "hi", title: "Tr 2", summary: "" },
        ];
        return buildSetListingParams(f);
      },
    ],
    ["slot times", () => slotFromLocalDatetimes("", "", "1")],
    ["slot order", () => slotFromLocalDatetimes("2030-01-01T11:00", "2030-01-01T10:00", "1")],
    ["slot capacity", () => slotFromLocalDatetimes("2030-01-01T10:00", "2030-01-01T11:00", "")],
  ];

  it.each(THROWS)("words %s in Hindi with no English left", async (_name, run) => {
    await activateWith("hi", { hi: async () => HINDI });
    let message = "";
    try {
      run();
    } catch (err) {
      expect(err).toBeInstanceOf(ListingInputError);
      message = (err as Error).message;
    }
    expect(message).toMatch(/[ऀ-ॿ]/);
    // The currency code is what the person typed, so it is data.
    expect(leftoverLatin(message.replace("ZZZ", ""))).toEqual([]);
  });

  it("words the same case differently in English", async () => {
    const english = () => {
      try {
        withBlock("booking", { lead_time_secs: "" })();
      } catch (err) {
        return (err as Error).message;
      }
      return "";
    };
    expect(english()).toBe("lead time is required");
  });
});
