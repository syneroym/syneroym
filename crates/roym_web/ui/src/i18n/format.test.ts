import { afterEach, describe, expect, it } from "vitest";
import { formatAge, formatDateTime, formatNumber, formatUtcDate, timeElement } from "./format";
import { activateWith, activate } from "./i18n";

afterEach(async () => {
  await activate("en");
});

const HINDI = { hi: async () => "age-moments = अभी-अभी" };

describe("formatNumber", () => {
  it("groups in the Indian way with Latin digits in both languages", async () => {
    expect(formatNumber(100000.5)).toBe("1,00,000.5");
    await activateWith("hi", HINDI);
    expect(formatNumber(100000.5)).toBe("1,00,000.5");
  });
});

describe("formatDateTime", () => {
  const secs = 1800000000;

  it("uses the local zone name and the year", () => {
    const text = formatDateTime(secs);
    expect(text).toContain("2027");
    expect(text).toContain("UTC");
    const expected = new Intl.DateTimeFormat("en-IN-u-nu-latn", {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      timeZoneName: "short",
    }).format(new Date(secs * 1000));
    expect(text).toBe(expected);
  });

  it("follows the Hub language", async () => {
    const english = formatDateTime(secs);
    await activateWith("hi", HINDI);
    expect(formatDateTime(secs)).not.toBe(english);
  });

  it("throws a RangeError for a value that is not a date", () => {
    expect(() => formatDateTime(Number.NaN)).toThrow(RangeError);
  });
});

describe("formatUtcDate", () => {
  it("spells the UTC date", () => {
    expect(formatUtcDate(1790000000)).toBe("21 Sept 2026");
  });
});

describe("formatAge", () => {
  it("uses moments, minutes, hours and days", () => {
    expect(formatAge(30)).toBe("moments ago");
    expect(formatAge(300)).toBe("5 minutes ago");
    expect(formatAge(7200)).toBe("2 hours ago");
    expect(formatAge(3 * 86400)).toBe("3 days ago");
  });

  it("is not English in Hindi", async () => {
    await activateWith("hi", HINDI);
    expect(formatAge(30)).toBe("अभी-अभी");
    expect(formatAge(300)).not.toMatch(/ago/);
  });
});

describe("timeElement", () => {
  it("holds the ISO value computed from the number", () => {
    const el = timeElement(1800000000);
    expect(el.tagName).toBe("TIME");
    expect(el.getAttribute("datetime")).toBe("2027-01-15T08:00:00.000Z");
    expect(el.textContent).toBe(formatDateTime(1800000000));
  });

  it("throws a RangeError for a value that is not a date", () => {
    expect(() => timeElement(Number.NaN)).toThrow(RangeError);
  });
});
