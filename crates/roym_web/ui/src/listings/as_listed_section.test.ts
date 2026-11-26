import { afterEach, describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import { asListedSection } from "./as_listed_section";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("asListedSection", () => {
  it("fills empty deal terms from saved deal defaults", async () => {
    stubRpc({
      "transaction.signing-status": () => ({ certificate: { state: "installed" } }),
      "deal.defaults-get": () => ({
        cancellation_terms: { lang: "en", text: "24h cancellation" },
        refund_terms: { lang: "en", text: "Pro-rata refund" },
        dispute_path: { lang: "en", text: "Local mediation" },
        quote_valid_secs: 7200,
      }),
    });

    const section = asListedSection();
    await vi.waitFor(() => {
      const read = section.read();
      expect(read.cancellation_terms).toBe("24h cancellation");
    });

    const read = section.read();
    expect(read.refund_terms).toBe("Pro-rata refund");
    expect(read.dispute_path).toBe("Local mediation");
    expect(read.quote_valid_hours).toBe("2");
  });
});
