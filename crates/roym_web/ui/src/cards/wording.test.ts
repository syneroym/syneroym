import { describe, expect, it } from "vitest";
import { OPEN_TO_LABELS, SERVICE_LOCATION_EDITOR_LABELS } from "../listings/labels";
import { renderCard } from "./render";
import * as wording from "./wording";

// A deal has a quote author and a counterparty. Nothing a person reads on a
// card may say "provider", "consumer" or "customer" instead.
const OLD_WORDS = /provider|consumer|customer/i;

const ROLES = ["counterparty", "quote-author"] as const;
const PLACES = ["at-giver", "at-receiver", "remote"] as const;
const DID_A = "did:key:zCounterparty";
const DID_B = "did:key:zQuoteAuthor";

function terms(where: (typeof PLACES)[number]) {
  return {
    scope: "Trim the hedge",
    currency: "EUR",
    amount_minor: 12000,
    tax_minor: 2000,
    fees_minor: 100,
    payee: "A. Gardener",
    payment_methods: ["cash"],
    payment_timing: "before-work" as const,
    schedule: { earliest_secs: 1700000000, latest_secs: 1700086400 },
    location: { where, address: "1 Garden Lane" },
    cancellation_terms: "Free until a day before",
    refund_terms: "Refund if the work is not done",
    dispute_path: "Write to the quote author",
    quote_expires_at_secs: 1700090000,
  };
}

function textOf(card: { type: string; data?: unknown }): string {
  return renderCard({ version: 1, ...card }).textContent ?? "";
}

describe("the words on deal cards", () => {
  it("keeps the shared notices free of the old words", () => {
    for (const [name, value] of Object.entries(wording)) {
      if (typeof value === "string") {
        expect(value, `wording.${name}`).not.toMatch(OLD_WORDS);
      }
    }
  });

  it("keeps the labels for places and audiences free of the old words", () => {
    for (const label of [
      ...Object.values(SERVICE_LOCATION_EDITOR_LABELS),
      ...Object.values(OPEN_TO_LABELS),
    ]) {
      expect(label).not.toMatch(OLD_WORDS);
    }
  });

  it("keeps a quote and an agreement free of the old words for every place and role", () => {
    for (const where of PLACES) {
      for (const role of ROLES) {
        const common = { counterparty_did: DID_A, quote_author_did: DID_B, terms: terms(where) };
        const quote = textOf({ type: "quote", data: { ...common, quote_id: "quo_1", listing_id: "lst_1" } });
        expect(quote).not.toMatch(OLD_WORDS);
        for (const pair of ["complete", { state: "half", role }, role] as const) {
          const agreement = textOf({
            type: "agreement-receipt",
            data: { ...common, quote_record_id: "rec_quote", role, pair },
          });
          expect(agreement).not.toMatch(OLD_WORDS);
        }
      }
    }
  });

  it("keeps the payment and work cards free of the old words for every role", () => {
    for (const role of ROLES) {
      const base = { agreement: "rec_agreement", counterparty_did: DID_A, quote_author_did: DID_B, role };
      const ack = textOf({
        type: "payment-acknowledgement",
        data: { ...base, currency: "EUR", amount_minor: 12000, observed_at_secs: 1700000000, method: "cash", reference: "7" },
      });
      const fulfilment = textOf({ type: "fulfilment-receipt", data: { ...base, terms: terms("remote") } });
      expect(ack).not.toMatch(OLD_WORDS);
      expect(fulfilment).not.toMatch(OLD_WORDS);
    }
    const request = textOf({
      type: "payment-request",
      data: { agreement: "rec_agreement", counterparty_did: DID_A, quote_author_did: DID_B, currency: "EUR", amount_minor: 12000, note: "Please pay" },
    });
    expect(request).not.toMatch(OLD_WORDS);
  });

  it("keeps a booking card free of the old words in every state", () => {
    const states = ["scheduled", "in-progress", "completed", "cancelled", "conflict", "ended-unconfirmed"];
    const tracks = ["none", "claimed", "acknowledged", "unconfirmed"];
    for (const state of states) {
      for (const conflict of ["slot-taken", "slot-unavailable"]) {
        for (const payment of tracks) {
          for (const fulfilment of tracks) {
            for (const cancelled_by of ROLES) {
              const text = textOf({
                type: "booking-progress",
                data: {
                  agreement: "rec_agreement",
                  counterparty_did: DID_A,
                  quote_author_did: DID_B,
                  seq: 2,
                  state,
                  conflict,
                  payment,
                  fulfilment,
                  track_window_ends_at_secs: 1700000000,
                  cancelled_by,
                  cancel_reason: "changed my mind",
                },
              });
              expect(text).not.toMatch(OLD_WORDS);
            }
          }
        }
      }
    }
  });

  it("keeps a request card free of the old words", () => {
    const text = textOf({
      type: "request",
      data: { description: "Trim the hedge", data_use_notice: wording.PAYMENT_NOTICE, categories: ["gardening"] },
    });
    expect(text).not.toMatch(OLD_WORDS);
  });
});
