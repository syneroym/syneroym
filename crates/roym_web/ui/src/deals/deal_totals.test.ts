import { describe, expect, it } from "vitest";
import { dealTotals } from "./deal_totals";

describe("dealTotals", () => {
  it("calculates totals for quote-author", () => {
    const commitments = [
      { giver: "quote-author", declared_value_minor: 500 },
      { giver: "counterparty", declared_value_minor: 800 },
    ];
    const res = dealTotals(commitments, "quote-author");
    expect(res.give).toBe(500n);
    expect(res.receive).toBe(800n);
    expect(res.difference).toBe(300n);
  });

  it("calculates totals for counterparty", () => {
    const commitments = [
      { giver: "quote-author", declared_value_minor: 500 },
      { giver: "counterparty", declared_value_minor: 800 },
    ];
    const res = dealTotals(commitments, "counterparty");
    expect(res.give).toBe(800n);
    expect(res.receive).toBe(500n);
    expect(res.difference).toBe(-300n);
  });

  it("handles equal totals resulting in zero difference", () => {
    const commitments = [
      { giver: "quote-author", declared_value_minor: 1200 },
      { giver: "counterparty", declared_value_minor: 1200 },
    ];
    const res = dealTotals(commitments, "quote-author");
    expect(res.give).toBe(1200n);
    expect(res.receive).toBe(1200n);
    expect(res.difference).toBe(0n);
  });

  it("handles values above 2^53 with BigInt precision", () => {
    const large1 = 10_000_000_000_000_000n; // 10^16, above 2^53 (9 * 10^15)
    const large2 = 12_000_000_000_000_000n;
    const commitments = [
      { giver: "quote-author", declared_value_minor: large1 },
      { giver: "counterparty", declared_value_minor: large2 },
    ];
    const res = dealTotals(commitments, "quote-author");
    expect(res.give).toBe(large1);
    expect(res.receive).toBe(large2);
    expect(res.difference).toBe(2_000_000_000_000_000n);
  });
});
