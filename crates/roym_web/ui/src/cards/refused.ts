/// A card whose signature or envelope could not be verified by this node.
/// Shown so the person knows a card was received, but never rendered as
/// a valid card or treated as an offer.
export function renderRefusedCard(type: string, version: number, reason?: string): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-refused";
  card.setAttribute("data-verified", "false");

  const title = document.createElement("h3");
  title.textContent = `Unverified ${type} card (v${version})`;
  card.appendChild(title);

  const desc = document.createElement("p");
  desc.className = "refused-notice";
  desc.textContent =
    "This node received a card of this type, but could not verify it. " +
    "It is shown so you know it was sent, but cannot be accepted or acted upon.";
  card.appendChild(desc);

  if (reason) {
    const reasonEl = document.createElement("p");
    reasonEl.className = "refused-reason";
    reasonEl.textContent = `Reason: ${reason}`;
    card.appendChild(reasonEl);
  }

  return card;
}
