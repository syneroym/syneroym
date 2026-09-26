import { describe, expect, it } from "vitest";
import { formatMinor } from "../../money.js";
import { renderRefusedCard } from "../refused.js";
import { renderAgreementReceipt } from "./agreement_receipt.js";
import { renderBookingProgress } from "./booking_progress.js";
import { renderFulfilmentReceipt } from "./fulfilment_receipt.js";
import { renderPaymentAcknowledgement } from "./payment_acknowledgement.js";
import { renderPaymentRequest } from "./payment_request.js";
import { renderQuote } from "./quote.js";
import { renderRequest } from "./request.js";

describe("Card templates", () => {
  describe("renderRequest", () => {
    it("renders every field it is given as text", () => {
      const data = {
        description: "Need roof repair after storm",
        categories: ["roofing", "repair"],
        listing_id: "list-123",
        window: {
          earliest_secs: 1700000000,
          latest_secs: 1700086400,
        },
        data_use_notice: "Standard data use notice text",
      };
      const el = renderRequest(data);

      expect(el.classList.contains("card-request")).toBe(true);
      expect(el.textContent).toContain("Need roof repair after storm");
      expect(el.textContent).toContain("roofing, repair");
      expect(el.textContent).toContain("list-123");
      expect(el.textContent).toContain(new Date(1700000000 * 1000).toISOString());
      expect(el.textContent).toContain(new Date(1700086400 * 1000).toISOString());
      expect(el.textContent).toContain("Standard data use notice text");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        description: xss,
        categories: [xss],
        listing_id: xss,
        data_use_notice: xss,
      };
      const el = renderRequest(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });
  });

  describe("renderQuote", () => {
    it("renders every field it is given as text", () => {
      const data = {
        scope: "Install composite roofing tiles",
        currency: "USD",
        amount_minor: 450000,
        tax_minor: 36000,
        fees_minor: 5000,
        payee: "https://pay.example.com/invoice/99",
        payment_timing: "before-work" as const,
        schedule: {
          earliest_secs: 1700000000,
          latest_secs: 1700086400,
        },
        location: {
          where: "at-customer" as const,
          address: "123 Main Street, Town",
        },
        cancellation_terms: "Full refund up to 24 hours prior",
        refund_terms: "Pro-rated refund for uncompleted work",
        dispute_path: "https://disputes.example.com/arbitrate",
        quote_expires_at_secs: 1700100000,
      };
      const el = renderQuote(data);

      expect(el.classList.contains("card-quote")).toBe(true);
      expect(el.textContent).toContain("Install composite roofing tiles");
      expect(el.textContent).toContain("4500.00 USD");
      expect(el.textContent).toContain("360.00 USD");
      expect(el.textContent).toContain("50.00 USD");
      expect(el.textContent).toContain("Payment before the work");
      expect(el.textContent).toContain("123 Main Street, Town");
      expect(el.textContent).toContain("Full refund up to 24 hours prior");
      expect(el.textContent).toContain("Pro-rated refund for uncompleted work");
      expect(el.textContent).toContain(new Date(1700100000 * 1000).toISOString());
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        scope: xss,
        currency: "USD",
        amount_minor: 1000,
        payee: xss,
        cancellation_terms: xss,
        refund_terms: xss,
        dispute_path: xss,
        location: {
          where: "at-customer" as const,
          address: xss,
        },
      };
      const el = renderQuote(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: payee as a text node, not an anchor", () => {
      const el = renderQuote({
        currency: "USD",
        amount_minor: 1000,
        payee: "javascript:alert(document.cookie)",
      });

      const anchors = el.querySelectorAll(".quote-payee a");
      expect(anchors.length).toBe(0);
      expect(el.querySelector(".quote-payee")?.textContent).toContain("javascript:alert(document.cookie)");
    });
  });

  describe("renderAgreementReceipt", () => {
    it("renders every field it is given and never says verified", () => {
      const data = {
        role: "consumer" as const,
        consumer_did: "did:key:zConsumer123",
        provider_did: "did:key:zProvider456",
        quote_record_id: "rec-quote-789",
        pair_state: "complete" as const,
        scope: "Plumbing repair",
        currency: "EUR",
        amount_minor: 25000,
        tax_minor: 2500,
        fees_minor: 0,
        payee: "https://pay.example.com/receipt/1",
        payment_timing: "after-work" as const,
        cancellation_terms: "Non-refundable deposit",
        refund_terms: "No refunds on completed labor",
        dispute_path: "contact support@example.com",
      };
      const el = renderAgreementReceipt(data);

      expect(el.classList.contains("card-agreement-receipt")).toBe(true);
      expect(el.textContent).toContain("Both parties have signed these terms.");
      expect(el.textContent).toContain("did:key:zConsumer123");
      expect(el.textContent).toContain("did:key:zProvider456");
      expect(el.textContent).toContain("rec-quote-789");
      expect(el.textContent).toContain("Plumbing repair");
      expect(el.textContent).toContain("250.00 EUR");
      expect(el.textContent).toContain("Payment after the work");
      expect(el.textContent).toContain("Non-refundable deposit");
      expect(el.textContent).toContain("No refunds on completed labor");

      // Rule: Never says "verified"
      expect(el.textContent?.toLowerCase()).not.toContain("verified");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        consumer_did: xss,
        provider_did: xss,
        quote_record_id: xss,
        scope: xss,
        currency: "USD",
        amount_minor: 1000,
        payee: xss,
        cancellation_terms: xss,
        refund_terms: xss,
        dispute_path: xss,
      };
      const el = renderAgreementReceipt(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: payee as a text node, not an anchor", () => {
      const el = renderAgreementReceipt({
        currency: "USD",
        amount_minor: 1000,
        payee: "javascript:alert(1)",
      });

      const anchors = el.querySelectorAll(".receipt-payee a");
      expect(anchors.length).toBe(0);
      expect(el.querySelector(".receipt-payee")?.textContent).toContain("javascript:alert(1)");
    });
  });

  describe("renderPaymentRequest", () => {
    it("renders fields and never says verified", () => {
      const data = {
        currency: "USD",
        amount_minor: 5000,
        agreement_payee: "did:key:zProvider123",
        note: "Initial deposit",
      };
      const el = renderPaymentRequest(data);

      expect(el.classList.contains("card-payment-request")).toBe(true);
      expect(el.textContent).toContain("50.00 USD");
      expect(el.textContent).toContain("did:key:zProvider123");
      expect(el.textContent).toContain("Initial deposit");
      expect(el.textContent?.toLowerCase()).not.toContain("verif");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        currency: "USD",
        amount_minor: 1000,
        agreement_payee: xss,
        note: xss,
      };
      const el = renderPaymentRequest(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: payee as a text node, not an anchor", () => {
      const el = renderPaymentRequest({
        currency: "USD",
        amount_minor: 1000,
        agreement_payee: "javascript:alert(1)",
      });

      const anchors = el.querySelectorAll(".payment-request-payee a");
      expect(anchors.length).toBe(0);
      expect(el.querySelector(".payment-request-payee")?.textContent).toContain("javascript:alert(1)");
    });
  });

  describe("renderPaymentAcknowledgement", () => {
    it("renders fields and never says verified", () => {
      const data = {
        role: "provider" as const,
        currency: "EUR",
        amount_minor: 12500,
        observed_at_secs: 1700000000,
        method: "bank-transfer",
        reference: "TX-9988",
      };
      const el = renderPaymentAcknowledgement(data);

      expect(el.classList.contains("card-payment-acknowledgement")).toBe(true);
      expect(el.textContent).toContain("125.00 EUR");
      expect(el.textContent).toContain("bank-transfer");
      expect(el.textContent).toContain("TX-9988");
      expect(el.textContent).toContain(new Date(1700000000 * 1000).toISOString());
      expect(el.textContent?.toLowerCase()).not.toContain("verif");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        role: "consumer" as const,
        currency: "USD",
        amount_minor: 1000,
        method: xss,
        reference: xss,
      };
      const el = renderPaymentAcknowledgement(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: reference without anchor", () => {
      const el = renderPaymentAcknowledgement({
        reference: "javascript:alert(1)",
      });

      expect(el.querySelectorAll("a").length).toBe(0);
      expect(el.textContent).toContain("javascript:alert(1)");
    });
  });

  describe("renderFulfilmentReceipt", () => {
    it("renders fields and never says verified", () => {
      const data = {
        role: "provider" as const,
        terms: {
          scope: "Replaced faulty electrical panel",
        },
      };
      const el = renderFulfilmentReceipt(data);

      expect(el.classList.contains("card-fulfilment-receipt")).toBe(true);
      expect(el.textContent).toContain("Replaced faulty electrical panel");
      expect(el.textContent?.toLowerCase()).not.toContain("verif");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        role: "consumer" as const,
        terms: {
          scope: xss,
        },
      };
      const el = renderFulfilmentReceipt(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: scope without anchor", () => {
      const el = renderFulfilmentReceipt({
        terms: {
          scope: "javascript:alert(1)",
        },
      });

      expect(el.querySelectorAll("a").length).toBe(0);
      expect(el.textContent).toContain("javascript:alert(1)");
    });
  });

  describe("renderBookingProgress", () => {
    it("renders fields and never says verified", () => {
      const data = {
        state: "in-progress" as const,
        payment: "acknowledged" as const,
        fulfilment: "claimed" as const,
        cancel_reason: "Rescheduled by mutual agreement",
      };
      const el = renderBookingProgress(data);

      expect(el.classList.contains("card-booking-progress")).toBe(true);
      expect(el.textContent).toContain("In progress");
      expect(el.textContent).toContain("Rescheduled by mutual agreement");
      expect(el.textContent?.toLowerCase()).not.toContain("verif");
    });

    it("renders malicious payload as plain text with no img elements", () => {
      const xss = "<img src=x onerror=alert(1)>";
      const data = {
        state: "cancelled" as const,
        cancel_reason: xss,
      };
      const el = renderBookingProgress(data);

      expect(el.querySelectorAll("img").length).toBe(0);
      expect(el.textContent).toContain(xss);
    });

    it("renders javascript: cancel reason without anchor", () => {
      const el = renderBookingProgress({
        cancel_reason: "javascript:alert(1)",
      });

      expect(el.querySelectorAll("a").length).toBe(0);
      expect(el.textContent).toContain("javascript:alert(1)");
    });
  });

  describe("formatMinor", () => {
    it("formats exponent 0, 2, and 3 currencies", () => {
      expect(formatMinor(1500, "JPY")).toBe("1500 JPY");
      expect(formatMinor(1500, "USD")).toBe("15.00 USD");
      expect(formatMinor(1500, "KWD")).toBe("1.500 KWD");
    });
  });

  describe("renderRefusedCard", () => {
    it("carries data-verified=false and never the type's own class", () => {
      const el = renderRefusedCard("quote", 1, "tampered signature bytes");

      expect(el.getAttribute("data-verified")).toBe("false");
      expect(el.classList.contains("card-refused")).toBe(true);
      expect(el.classList.contains("card-quote")).toBe(false);
      expect(el.classList.contains("card-request")).toBe(false);
      expect(el.classList.contains("card-agreement-receipt")).toBe(false);
      expect(el.textContent).toContain("Unverified quote card (v1)");
      expect(el.textContent).toContain("tampered signature bytes");
    });
  });
});
