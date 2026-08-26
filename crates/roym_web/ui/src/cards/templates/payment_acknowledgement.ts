export interface PaymentAcknowledgementData {
  receipt_id?: string;
}

export function renderPaymentAcknowledgement(data?: PaymentAcknowledgementData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-payment-acknowledgement";
  const title = document.createElement("h3");
  title.textContent = "Payment Acknowledgement";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.receipt_id ? `Receipt ID: ${data.receipt_id}` : "Payment Acknowledgement v1";
  div.appendChild(p);
  return div;
}
