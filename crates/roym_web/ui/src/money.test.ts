import { describe, expect, it } from "vitest";
import {
  MoneyInputError,
  currencyMinorExponent,
  formatMinor,
  toMinorUnits,
} from "./money.js";

describe("currencyMinorExponent", () => {
  it("returns 0 for exponent-0 currencies", () => {
    expect(currencyMinorExponent("JPY")).toBe(0);
    expect(currencyMinorExponent("BIF")).toBe(0);
    expect(currencyMinorExponent("XOF")).toBe(0);
    expect(currencyMinorExponent(" jpy ")).toBe(0);
  });

  it("returns 3 for exponent-3 currencies", () => {
    expect(currencyMinorExponent("KWD")).toBe(3);
    expect(currencyMinorExponent("BHD")).toBe(3);
    expect(currencyMinorExponent("OMR")).toBe(3);
    expect(currencyMinorExponent(" kwd ")).toBe(3);
  });

  it("defaults to 2 for all other accepted currencies", () => {
    expect(currencyMinorExponent("USD")).toBe(2);
    expect(currencyMinorExponent("EUR")).toBe(2);
    expect(currencyMinorExponent("GBP")).toBe(2);
  });

  it("returns undefined for unknown currency codes", () => {
    expect(currencyMinorExponent("XYZ")).toBeUndefined();
    expect(currencyMinorExponent("")).toBeUndefined();
    expect(currencyMinorExponent("USDX")).toBeUndefined();
  });
});

describe("toMinorUnits", () => {
  it("converts strings with varying exponents to integer minor units", () => {
    expect(toMinorUnits("")).toBeUndefined();
    expect(toMinorUnits("   ")).toBeUndefined();

    // Exponent 2 (default)
    expect(toMinorUnits("35.50")).toBe(3550);
    expect(toMinorUnits("35.5")).toBe(3550);
    expect(toMinorUnits("35")).toBe(3500);
    expect(toMinorUnits("0.05")).toBe(5);
    expect(toMinorUnits("0")).toBe(0);

    // Exponent 0
    expect(toMinorUnits("1200", 0)).toBe(1200);
    expect(toMinorUnits("0", 0)).toBe(0);

    // Exponent 3
    expect(toMinorUnits("2.5", 3)).toBe(2500);
    expect(toMinorUnits("2.500", 3)).toBe(2500);
    expect(toMinorUnits("2.123", 3)).toBe(2123);
  });

  it("throws MoneyInputError for malformed input or too many decimal places", () => {
    expect(() => toMinorUnits("35.555", 2)).toThrow(MoneyInputError);
    expect(() => toMinorUnits("12.5", 0)).toThrow(MoneyInputError);
    expect(() => toMinorUnits("abc", 2)).toThrow(MoneyInputError);
    expect(() => toMinorUnits("-10", 2)).toThrow(MoneyInputError);
  });
});

describe("formatMinor", () => {
  it("formats exponent-0 currencies with no decimal separator", () => {
    expect(formatMinor(1200, "JPY")).toBe("1200 JPY");
    expect(formatMinor(0, "JPY")).toBe("0 JPY");
    expect(formatMinor(-500, "JPY")).toBe("-500 JPY");
  });

  it("formats exponent-2 currencies with two decimal places", () => {
    expect(formatMinor(3550, "USD")).toBe("35.50 USD");
    expect(formatMinor(3500, "EUR")).toBe("35.00 EUR");
    expect(formatMinor(5, "USD")).toBe("0.05 USD");
    expect(formatMinor(0, "USD")).toBe("0.00 USD");
    expect(formatMinor(-3550, "USD")).toBe("-35.50 USD");
  });

  it("formats exponent-3 currencies with three decimal places", () => {
    expect(formatMinor(2500, "KWD")).toBe("2.500 KWD");
    expect(formatMinor(123, "KWD")).toBe("0.123 KWD");
    expect(formatMinor(5, "BHD")).toBe("0.005 BHD");
    expect(formatMinor(0, "OMR")).toBe("0.000 OMR");
    expect(formatMinor(-2500, "KWD")).toBe("-2.500 KWD");
  });

  it("rounds non-integer minor unit inputs rather than generating malformed decimals", () => {
    expect(formatMinor(3550.7, "USD")).toBe("35.51 USD");
    expect(formatMinor(3550.2, "USD")).toBe("35.50 USD");
    expect(formatMinor(1200.4, "JPY")).toBe("1200 JPY");
  });
});
