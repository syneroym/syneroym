import { describe, expect, it } from "vitest";
import { SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA } from "../extensions/registry";
import { buildDealTerms } from "./quote_terms";

interface TestCommitment {
  giver?: string;
  kind?: string;
  resource_kind?: string;
  after?: string[];
  description?: { lang?: string; text?: string };
  declared_value_minor?: number;
  location?: unknown;
  window?: unknown;
  extension?: unknown;
  details?: unknown;
  payee?: string;
}

describe("buildDealTerms", () => {
  it("maps terms into deal commitments and payment dependencies", () => {
    // 1. Amount 0: single commitment, no c2
    const zeroDeal = buildDealTerms(
      {
        scope: "Free consultation",
        currency: "EUR",
        amountMinor: 0,
        payee: "Consultant",
        timing: "after-work",
        where: "remote",
      },
      "en",
    );
    expect(zeroDeal.currency).toBe("EUR");
    const zeroComms = zeroDeal.commitments as TestCommitment[];
    expect(zeroComms).toHaveLength(1);
    expect(zeroComms[0].giver).toBe("quote-author");
    expect(zeroComms[0].declared_value_minor).toBe(0);
    expect(zeroComms[0].after).toEqual([]);

    // 2. Before-work: c1 after c2
    const beforeDeal = buildDealTerms(
      {
        scope: "Guitar lesson",
        currency: "USD",
        amountMinor: 5000,
        payee: "Teacher",
        timing: "before-work",
        where: "at-giver",
      },
      "en",
    );
    const beforeComms = beforeDeal.commitments as TestCommitment[];
    expect(beforeComms).toHaveLength(2);
    expect(beforeComms[0].after).toEqual(["c2"]);
    expect(beforeComms[1].after).toEqual([]);

    // 3. After-work: c2 after c1
    const afterDeal = buildDealTerms(
      {
        scope: "Guitar lesson",
        currency: "USD",
        amountMinor: 5000,
        payee: "Teacher",
        timing: "after-work",
        where: "at-giver",
      },
      "en",
    );
    const afterComms = afterDeal.commitments as TestCommitment[];
    expect(afterComms).toHaveLength(2);
    expect(afterComms[0].after).toEqual([]);
    expect(afterComms[1].after).toEqual(["c1"]);

    // 4. Slot commitment
    const slotDeal = buildDealTerms(
      {
        scope: "Clean windows",
        currency: "EUR",
        amountMinor: 8000,
        payee: "Cleaner",
        timing: "after-work",
        where: "at-receiver",
        listingId: "listing-123",
        slotId: "slot-123",
        schedule: { earliest_secs: 1800000000, latest_secs: 1800003600 },
      },
      "en",
    );
    const slotComms = slotDeal.commitments as TestCommitment[];
    expect(slotComms[0].resource_kind).toBe("service-in-slot");
    expect(slotComms[0].extension).toEqual({ id: SCHEDULING_EXTENSION, version: 1 });
    expect(slotComms[0].details).toEqual({
      extension: SCHEDULING_EXTENSION,
      schema: SCHEDULING_SLOT_SCHEMA,
      data: { listing_id: "listing-123", slot_id: "slot-123" },
    });
    expect(slotComms[0].window).toEqual({ earliest_secs: 1800000000, latest_secs: 1800003600 });
  });

  it("pre-fill never comes from text in another language", () => {
    const deal = buildDealTerms(
      {
        give: "Custom work description in English",
        scope: "Scope in English",
        currency: "INR",
        amountMinor: 10000,
        payee: "Bob",
        timing: "after-work",
        where: "remote",
      },
      "en",
    );
    const comms = deal.commitments as TestCommitment[];
    expect(comms[0].description?.lang).toBe("en");
    expect(comms[0].description?.text).toBe("Custom work description in English");
    expect((deal.scope as { lang?: string }).lang).toBe("en");
    expect(comms[1].description?.lang).toBe("en");
  });

  it("quotes in a language with no catalog build successfully with language-neutral currency description", () => {
    const deal = buildDealTerms(
      {
        scope: "Repair",
        currency: "INR",
        amountMinor: 10000,
        payee: "Repair Shop",
        timing: "after-work",
        where: "remote",
      },
      "ta",
    );
    const comms = deal.commitments as TestCommitment[];
    expect(comms[1].description?.lang).toBe("ta");
    expect(comms[1].description?.text).toBe("INR 100.00");
  });

  it("builds commitments from custom rows with multi-row after dependencies", () => {
    const deal = buildDealTerms(
      {
        scope: "Full renovation",
        currency: "INR",
        timing: "before-work",
        rows: [
          {
            giver: "quote-author",
            kind: "service",
            description: "Painting",
            declaredValueMinor: 3000,
            where: "at-receiver",
          },
          {
            giver: "quote-author",
            kind: "service",
            description: "Carpentry",
            declaredValueMinor: 4000,
            where: "at-receiver",
          },
          {
            giver: "counterparty",
            kind: "money",
            declaredValueMinor: 5000,
            payee: "Painter",
          },
          {
            giver: "counterparty",
            kind: "goods",
            description: "Paint cans",
            declaredValueMinor: 2000,
          },
        ],
      },
      "en",
    );

    const comms = deal.commitments as TestCommitment[];
    expect(comms).toHaveLength(4);
    // Before work: author services (c1, c2) happen after counterparty money (c3)
    expect(comms[0].after).toEqual(["c3"]);
    expect(comms[1].after).toEqual(["c3"]);
    expect(comms[2].after).toEqual([]); // money c3
    expect(comms[3].after).toEqual([]); // goods c4
  });
});
