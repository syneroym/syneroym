import { renderLink } from "../link.js";

export interface PaymentRequestData {
  amount?: string | number;
  currency?: string;
  url?: string;
}

export function renderPaymentRequest(data?: PaymentRequestData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-payment-request";
  const title = document.createElement("h3");
  title.textContent = "Payment Request";
  div.appendChild(title);
  const p = document.createElement("p");
  if (data?.amount !== undefined && data?.amount !== null && data?.amount !== "") {
    p.textContent = data.currency ? `Amount: ${data.amount} ${data.currency}` : `Amount: ${data.amount}`;
  } else {
    p.textContent = "Payment Request v1";
  }
  div.appendChild(p);

  if (data?.url) {
    const pLink = document.createElement("p");
    pLink.className = "payment-link";
    pLink.appendChild(renderLink(data.url));
    div.appendChild(pLink);
  }

  return div;
}


