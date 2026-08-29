export interface PaymentRequestData {
  amount?: string;
  currency?: string;
}

export function renderPaymentRequest(data?: PaymentRequestData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-payment-request";
  const title = document.createElement("h3");
  title.textContent = "Payment Request";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.amount ? `Amount: ${data.amount}` : "Payment Request v1";
  div.appendChild(p);
  return div;
}
