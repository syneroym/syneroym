import { describe, expect, it } from "vitest";
import { renderLink } from "./link";
import { renderCard } from "./render";
import { renderPaymentRequest } from "./templates/payment_request";

describe("renderLink", () => {
  it("renders an https URL as a link", () => {
    const node = renderLink("https://example.com/path");
    expect(node.nodeName).toBe("A");
    expect((node as HTMLAnchorElement).href).toBe("https://example.com/path");
  });

  it("renders an http URL as a link", () => {
    const node = renderLink("http://example.com/path");
    expect(node.nodeName).toBe("A");
  });

  it("rejects a javascript: URL as plain text", () => {
    const node = renderLink("javascript:alert(1)");
    expect(node.nodeName).toBe("#text");
    expect(node.textContent).toBe("javascript:alert(1)");
  });

  it("rejects a data: URL as plain text", () => {
    const node = renderLink("data:text/html,<script>alert(1)</script>");
    expect(node.nodeName).toBe("#text");
  });

  it("rejects a file: URL as plain text", () => {
    const node = renderLink("file:///etc/passwd");
    expect(node.nodeName).toBe("#text");
  });

  it("renders an unparseable string as plain text", () => {
    const node = renderLink("not a url");
    expect(node.nodeName).toBe("#text");
    expect(node.textContent).toBe("not a url");
  });

  it("renders payment-request card with a safe URL payee as a link", () => {
    const el = renderPaymentRequest({
      amount_minor: 10000,
      currency: "USD",
      agreement_payee: "https://pay.example.com/invoice1",
    });
    const link = el.querySelector(".payment-request-payee a") as HTMLAnchorElement | null;
    expect(link).not.toBeNull();
    expect(link?.href).toBe("https://pay.example.com/invoice1");
    expect(link?.textContent).toBe("https://pay.example.com/invoice1");
  });

  it("renders payment-request card via renderCard with javascript: payee as plain text", () => {
    const el = renderCard({
      type: "payment-request",
      version: 1,
      agreement_payee: "javascript:evil()",
      data: { amount_minor: 5000, currency: "USD" },
    });
    const link = el.querySelector(".payment-request-payee a");
    expect(link).toBeNull();
    const payeeP = el.querySelector(".payment-request-payee");
    expect(payeeP?.textContent).toContain("javascript:evil()");
  });
});
