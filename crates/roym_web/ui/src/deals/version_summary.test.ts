import { describe, expect, it } from "vitest";
import { type DealTerms, versionSummary } from "./version_summary";

describe("versionSummary", () => {
  it("detects added, removed, changed, and calculates totals", () => {
    const first: DealTerms = {
      commitments: [
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: { lang: "en", text: "Guitar lessons" },
          declared_value_minor: 1000,
        },
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: { lang: "en", text: "Piano tuning" },
          declared_value_minor: 500,
        },
        {
          giver: "counterparty",
          resource_kind: "currency",
          description: { lang: "en", text: "INR 1500" },
          declared_value_minor: 1500,
          payee: "did:key:zAuthor",
        },
      ],
    };

    const latest: DealTerms = {
      commitments: [
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: { lang: "en", text: "Guitar lessons" },
          declared_value_minor: 1200, // changed value
        },
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: { lang: "en", text: "Vocal coaching" }, // added
          declared_value_minor: 800,
        },
        // Piano tuning was removed
        {
          giver: "counterparty",
          resource_kind: "currency",
          description: { lang: "en", text: "INR 2000" }, // description changed from "INR 1500" -> treated as removed & added
          declared_value_minor: 2000,
          payee: "did:key:zAuthor",
        },
      ],
    };

    const summary = versionSummary(first, latest, "quote-author");

    // Mine (quote-author)
    expect(summary.mine.totalBefore).toBe(1500n);
    expect(summary.mine.totalAfter).toBe(2000n);
    expect(summary.mine.changed).toHaveLength(1);
    expect(summary.mine.changed[0].before.declared_value_minor).toBe(1000);
    expect(summary.mine.changed[0].after.declared_value_minor).toBe(1200);

    expect(summary.mine.added).toHaveLength(1);
    expect((summary.mine.added[0].description as { text: string }).text).toBe("Vocal coaching");

    expect(summary.mine.removed).toHaveLength(1);
    expect((summary.mine.removed[0].description as { text: string }).text).toBe("Piano tuning");

    // Theirs (counterparty) - description changed so 1 removed, 1 added
    expect(summary.theirs.totalBefore).toBe(1500n);
    expect(summary.theirs.totalAfter).toBe(2000n);
    expect(summary.theirs.changed).toHaveLength(0);
    expect(summary.theirs.removed).toHaveLength(1);
    expect(summary.theirs.added).toHaveLength(1);
  });

  it("treats changed description as one removed and one added", () => {
    const first: DealTerms = {
      commitments: [
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: "Old description",
          declared_value_minor: 500,
        },
      ],
    };

    const latest: DealTerms = {
      commitments: [
        {
          giver: "quote-author",
          resource_kind: "goods-or-service",
          description: "New description",
          declared_value_minor: 500,
        },
      ],
    };

    const summary = versionSummary(first, latest, "quote-author");
    expect(summary.mine.changed).toHaveLength(0);
    expect(summary.mine.removed).toHaveLength(1);
    expect(summary.mine.added).toHaveLength(1);
  });
});
