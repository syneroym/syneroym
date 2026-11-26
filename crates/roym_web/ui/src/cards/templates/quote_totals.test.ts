import { describe, expect, it } from "vitest";
import { renderQuote } from "./quote";
import { renderVersionSummary, type VersionSummary } from "../../deals/version_summary";

describe("quote totals block and version summary", () => {
  it("renders quote version number", () => {
    const el1 = renderQuote({ sequence: 1 });
    expect(el1.querySelector(".quote-version")?.textContent).toBe("Version 1");

    const el2 = renderQuote({ version_number: 3 });
    expect(el2.querySelector(".quote-version")?.textContent).toBe("Version 3");
  });

  it("renders equal totals with no difference line", () => {
    const el = renderQuote({
      terms: {
        currency: "INR",
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            kind: "fulfilment",
            resource_kind: "goods-or-service",
            description: "Violin lesson",
            declared_value_minor: 1000,
          },
          {
            commitment_id: "c2",
            giver: "counterparty",
            kind: "currency",
            resource_kind: "currency",
            description: "INR 10.00",
            declared_value_minor: 1000,
          },
        ],
      },
    });

    const block = el.querySelector(".quote-totals-block");
    expect(block).not.toBeNull();
    expect(el.querySelector(".quote-total")).toBeNull();

    const give = block?.querySelector(".quote-total-give");
    const receive = block?.querySelector(".quote-total-receive");
    const diff = block?.querySelector(".quote-total-diff");

    // From viewer (counterparty) perspective: they give c2 (1000), receive c1 (1000)
    expect(give?.textContent).toContain("You give: ₹10.00");
    expect(receive?.textContent).toContain("You receive: ₹10.00");
    expect(diff).toBeNull();
  });

  it("renders difference line when receive > give for counterparty", () => {
    const el = renderQuote({
      role: "counterparty",
      terms: {
        currency: "INR",
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            kind: "fulfilment",
            resource_kind: "goods-or-service",
            description: "Violin lesson",
            declared_value_minor: 1500,
          },
          {
            commitment_id: "c2",
            giver: "counterparty",
            kind: "currency",
            resource_kind: "currency",
            description: "INR 10.00",
            declared_value_minor: 1000,
          },
        ],
      },
    });

    const block = el.querySelector(".quote-totals-block");
    expect(block?.querySelector(".quote-total-give")?.textContent).toContain("You give: ₹10.00");
    expect(block?.querySelector(".quote-total-receive")?.textContent).toContain("You receive: ₹15.00");

    const diff = block?.querySelector(".quote-total-diff");
    expect(diff?.textContent).toContain("You receive ₹5.00 more than you give");
  });

  it("renders difference line when give > receive for quote-author", () => {
    const el = renderQuote({
      role: "quote-author",
      terms: {
        currency: "INR",
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            kind: "fulfilment",
            resource_kind: "goods-or-service",
            description: "Violin lesson",
            declared_value_minor: 2500,
          },
          {
            commitment_id: "c2",
            giver: "counterparty",
            kind: "currency",
            resource_kind: "currency",
            description: "INR 15.00",
            declared_value_minor: 1500,
          },
        ],
      },
    });

    const block = el.querySelector(".quote-totals-block");
    expect(block?.querySelector(".quote-total-give")?.textContent).toContain("You give: ₹25.00");
    expect(block?.querySelector(".quote-total-receive")?.textContent).toContain("You receive: ₹15.00");

    const diff = block?.querySelector(".quote-total-diff");
    expect(diff?.textContent).toContain("You give ₹10.00 more than you receive");
  });

  it("handles BigInt totals above 2^53", () => {
    const bigMinor = 10_000_000_000_000_000n; // 10^16, larger than 2^53 - 1
    const el = renderQuote({
      role: "quote-author",
      terms: {
        currency: "INR",
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            kind: "fulfilment",
            resource_kind: "goods-or-service",
            description: "Huge asset",
            declared_value_minor: bigMinor as unknown as number,
          },
        ],
      },
    });

    const give = el.querySelector(".quote-total-give");
    expect(give?.textContent).toContain("₹10,00,00,00,00,00,000.00");
  });

  it("falls back to legacy quote-total when no commitments array is provided", () => {
    const el = renderQuote({
      currency: "INR",
      amount_minor: 500,
    });
    expect(el.querySelector(".quote-totals-block")).toBeNull();
    expect(el.querySelector(".quote-total")?.textContent).toBe("Total: ₹5.00");
  });

  it("renders change summary via renderVersionSummary", () => {
    const summary: VersionSummary = {
      mine: {
        added: [
          {
            giver: "counterparty",
            resource_kind: "goods-or-service",
            description: "Additional tutoring",
          },
        ],
        removed: [],
        changed: [],
        totalBefore: 0n,
        totalAfter: 500n,
      },
      theirs: {
        added: [],
        removed: [
          {
            giver: "quote-author",
            resource_kind: "goods-or-service",
            description: "Old item",
          },
        ],
        changed: [
          {
            before: {
              giver: "quote-author",
              description: "Service fee",
              declared_value_minor: 100,
            },
            after: {
              giver: "quote-author",
              description: "Service fee",
              declared_value_minor: 200,
            },
          },
        ],
        totalBefore: 100n,
        totalAfter: 200n,
      },
    };

    const el = renderVersionSummary(summary);
    expect(el.className).toBe("quote-changes-summary");
    expect(el.querySelector(".quote-changes-heading")?.textContent).toBe("Changes since version 1");

    const added = el.querySelector(".quote-change-added");
    expect(added?.textContent).toBe("Added: Additional tutoring");

    const removed = el.querySelector(".quote-change-removed");
    expect(removed?.textContent).toBe("Removed: Old item");

    const changed = el.querySelector(".quote-change-changed");
    expect(changed?.textContent).toBe("Changed: Service fee");
  });

  it("renders summary when passed in QuoteData", () => {
    const summary: VersionSummary = {
      mine: {
        added: [{ giver: "counterparty", description: "Extra hour" }],
        removed: [],
        changed: [],
        totalBefore: 0n,
        totalAfter: 500n,
      },
      theirs: {
        added: [],
        removed: [],
        changed: [],
        totalBefore: 0n,
        totalAfter: 0n,
      },
    };

    const el = renderQuote({
      sequence: 2,
      summary,
    });

    expect(el.querySelector(".quote-changes-summary")).not.toBeNull();
    expect(el.querySelector(".quote-change-added")?.textContent).toBe("Added: Extra hour");
  });
});
