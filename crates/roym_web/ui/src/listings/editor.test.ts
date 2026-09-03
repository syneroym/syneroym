import { describe, expect, it } from "vitest";
import {
  buildSetListingParams,
  ListingInputError,
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
    categories: "",
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
        methods: "",
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
    expect(() => toMinorUnits("35.555")).toThrow(ListingInputError);
    expect(() => toMinorUnits("abc")).toThrow(ListingInputError);
    expect(() => toMinorUnits("-1")).toThrow(ListingInputError);
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
      categories: [],
      status: "active",
    });
  });

  it("emits an integer amount_minor for a decimal price and never a decimal anywhere", () => {
    const f = emptyForm();
    f.title = "Hedge trimming";
    f.categories = "Gardening, Outdoor";
    f.payment.enabled = true;
    f.payment.value.amount = "35.50";
    f.payment.value.fees = "2.5";
    f.payment.value.payee = "A. Gardener";
    f.booking.enabled = true;
    f.booking.value.mode = "slots";
    f.booking.value.lead_time_secs = "3600";
    f.service.enabled = true;
    f.service.value.duration_secs = "1800";

    const params = buildSetListingParams(f);
    const payment = params.payment as Record<string, unknown>;
    expect(payment.amount_minor).toBe(3550);
    expect(payment.fees_minor).toBe(250);
    expect(payment.currency).toBe("EUR");
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

  it("refuses a members-only relationship with no group DID", () => {
    const f = emptyForm();
    f.title = "x";
    f.relationship.enabled = true;
    f.relationship.value.open_to = "members";
    expect(() => buildSetListingParams(f)).toThrow(ListingInputError);
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
