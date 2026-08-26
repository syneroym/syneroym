export interface ContactOfferData {
  handle?: string;
  service?: string;
}

export function renderContactOffer(data?: ContactOfferData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-contact-offer";
  const title = document.createElement("h3");
  title.textContent = "Contact Offer";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.handle ? `Handle: ${data.handle}` : "Contact Offer v1";
  div.appendChild(p);
  return div;
}
