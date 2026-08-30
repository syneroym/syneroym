export interface QuoteData {
  price?: string | number;
  terms?: string;
}

export function renderQuote(data?: QuoteData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-quote";
  const title = document.createElement("h3");
  title.textContent = "Quote Card";
  div.appendChild(title);
  const p = document.createElement("p");
  if (data?.price !== undefined && data?.price !== null && data?.price !== "") {
    p.textContent = data.terms ? `Price: ${data.price} (${data.terms})` : `Price: ${data.price}`;
  } else {
    p.textContent = "Quote Card v1";
  }
  div.appendChild(p);
  return div;
}
