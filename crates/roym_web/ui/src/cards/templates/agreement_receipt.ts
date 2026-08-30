export interface AgreementReceiptData {
  agreement_id?: string;
}

export function renderAgreementReceipt(data?: AgreementReceiptData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-agreement-receipt";
  const title = document.createElement("h3");
  title.textContent = "Agreement Receipt";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent =
    data?.agreement_id !== undefined && data?.agreement_id !== null && data?.agreement_id !== ""
      ? `Agreement ID: ${data.agreement_id}`
      : "Agreement Receipt v1";
  div.appendChild(p);
  return div;
}
