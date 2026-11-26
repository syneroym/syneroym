import { describe, expect, it } from "vitest";
import { fillQuoteForm } from "./defaults";

describe("quote defaults fill order", () => {
  const builtIn = {
    authorDid: "did:key:zAuthorBuiltIn",
    cancellation: "Built-in cancellation",
    refund: "Built-in refund",
    dispute: "Built-in dispute",
  };

  it("uses built-in defaults and falls back to INR and author DID when listing and saved are empty", () => {
    const values = fillQuoteForm(null, null, builtIn);
    expect(values.currency).toBe("INR");
    expect(values.amountMinor).toBeUndefined();
    expect(values.timing).toBe("after-work");
    expect(values.where).toBe("at-receiver");
    expect(values.payee).toBe("did:key:zAuthorBuiltIn");
    expect(values.cancellation).toBe("Built-in cancellation");
    expect(values.refund).toBe("Built-in refund");
    expect(values.dispute).toBe("Built-in dispute");
    expect(values.quoteValidSecs).toBe(86400); // 24 hours
  });

  it("overrides built-in defaults with saved deal terms", () => {
    const saved = {
      cancellation_terms: "Saved cancellation",
      refund_terms: "Saved refund",
      dispute_path: "Saved dispute",
      quote_valid_secs: 172800, // 48 hours
    };

    const values = fillQuoteForm(null, saved, builtIn);
    expect(values.currency).toBe("INR");
    expect(values.payee).toBe("did:key:zAuthorBuiltIn");
    expect(values.cancellation).toBe("Saved cancellation");
    expect(values.refund).toBe("Saved refund");
    expect(values.dispute).toBe("Saved dispute");
    expect(values.quoteValidSecs).toBe(172800);
  });

  it("overrides saved deal terms and built-in with listing details", () => {
    const saved = {
      cancellation_terms: "Saved cancellation",
      refund_terms: "Saved refund",
      dispute_path: "Saved dispute",
      quote_valid_secs: 172800,
    };

    const listing = {
      payment: {
        currency: "USD",
        amount_minor: 4500,
        timing: "before-work" as const,
        payee: "did:key:zListingPayee",
      },
      deal_terms: {
        cancellation_terms: "Listing cancellation",
        refund_terms: "Listing refund",
        dispute_path: "Listing dispute",
        quote_valid_secs: 43200, // 12 hours
      },
      location: {
        where: "at-giver" as const,
        address: "123 Main St",
      },
    };

    const values = fillQuoteForm(listing, saved, builtIn);
    expect(values.currency).toBe("USD");
    expect(values.amountMinor).toBe(4500);
    expect(values.timing).toBe("before-work");
    expect(values.where).toBe("at-giver");
    expect(values.address).toBe("123 Main St");
    expect(values.payee).toBe("did:key:zListingPayee");
    expect(values.cancellation).toBe("Listing cancellation");
    expect(values.refund).toBe("Listing refund");
    expect(values.dispute).toBe("Listing dispute");
    expect(values.quoteValidSecs).toBe(43200);
  });
});
