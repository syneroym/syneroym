import { afterEach, describe, expect, it, vi } from "vitest";
import type { MergedHit } from "../directory/search";
import { activate } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { renderAsListedTap } from "./as_listed_tap";

const HIT = {
  listing_id: "lst_1",
  record_id: "rec_listing_1",
  issuer: "did:key:zOwner",
  title: "Hedge trimming",
  summary: "Neat hedges.",
  lang: "en",
  translations: [],
  categories: ["gardening"],
  conversation_address: "did:key:zOwnerConv",
  status: "active",
  verified: true,
  revocation_status: "unknown",
  age_secs: 10,
  sources: [],
  versions_differ: false,
  can_agree_as_listed: true,
} as unknown as MergedHit;

const PAYLOAD = {
  listing_id: "lst_1",
  lang: "en",
  acceptance: "confirm-first",
  payment: {
    currency: "EUR",
    model: "fixed",
    amount_minor: 3500,
    payee: "A. Gardener",
  },
  deal_terms: {
    cancellation_terms: { lang: "en", text: "Free until 24 hours before" },
    refund_terms: { lang: "en", text: "Full refund" },
    dispute_path: { lang: "en", text: "Write to us first" },
    quote_valid_secs: 3600,
    payment_timing: "after-work",
  },
};

describe("agree as listed tap", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("shows every term and sends the consent with no description", async () => {
    let sent: Record<string, unknown> | undefined;
    stubRpc({
      "directory.run-envelope": () => ({ envelope: "{}" }),
      "listing.verify": () => ({ verified: true, payload: PAYLOAD }),
      "conversation.open": () => ({ conversation_id: "conv_1" }),
      "enquiry.set": (params) => {
        sent = params as Record<string, unknown>;
        return {};
      },
    });
    const host = document.createElement("div");
    renderAsListedTap(host, HIT, "run", async () => {});

    await vi.waitFor(() => expect(host.querySelector("button.as-listed-send")).not.toBeNull());
    const copy = host.textContent ?? "";
    expect(copy).toContain("Price:");
    expect(copy).toContain("Will be paid to A. Gardener");
    expect(copy).toContain("Free until 24 hours before");
    expect(copy).toContain("Full refund");
    expect(copy).toContain("Write to us first");
    expect(copy).toContain("After the work");
    expect(copy).toContain("This consent holds for 48 hours");

    host.querySelector<HTMLButtonElement>("button.as-listed-send")!.click();
    await vi.waitFor(() => expect(sent).toBeDefined());
    const asListed = sent?.["as_listed"] as Record<string, unknown>;
    expect(asListed["valid_for_secs"]).toBe(48 * 3600);
    expect(asListed["viewed_lang"]).toBe("en");
    expect(asListed["viewed_via_mt"]).toBe(false);
    expect("description" in (sent ?? {})).toBe(false);
  });

  it("shows Ask alone on an offering that books a slot", async () => {
    stubRpc({
      "directory.run-envelope": () => ({ envelope: "{}" }),
      "listing.verify": () => ({
        verified: true,
        payload: { ...PAYLOAD, extension: { id: "scheduling", version: 1 } },
      }),
    });
    const host = document.createElement("div");
    renderAsListedTap(host, HIT, "run", async () => {});
    await vi.waitFor(() => expect(host.textContent).toContain("Ask first"));
    expect(host.querySelector("button.as-listed-send")).toBeNull();
  });
});
