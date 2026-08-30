export interface FulfilmentReceiptData {
  tracking?: string;
}

export function renderFulfilmentReceipt(data?: FulfilmentReceiptData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-fulfilment-receipt";
  const title = document.createElement("h3");
  title.textContent = "Fulfilment Receipt";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent =
    data?.tracking !== undefined && data?.tracking !== null && data?.tracking !== ""
      ? `Tracking: ${data.tracking}`
      : "Fulfilment Receipt v1";
  div.appendChild(p);
  return div;
}
