export interface QuoteData {
  price?: string;
  terms?: string;
}

export function renderQuote(data?: QuoteData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-quote";
  const title = document.createElement("h3");
  title.textContent = "Quote Card";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.price ? `Price: ${data.price}` : "Quote Card v1";
  div.appendChild(p);
  return div;
}
