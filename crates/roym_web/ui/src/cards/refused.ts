import { t } from "../i18n/i18n";

/// A card whose signature or envelope could not be verified by this node.
/// Shown so the person knows a card was received, but never rendered as
/// a valid card or treated as an offer.
export function renderRefusedCard(type: string, version: number, reason?: string): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-refused";
  card.setAttribute("data-verified", "false");

  const title = document.createElement("h3");
  title.textContent = t("card-refused-title", { type, version: String(version) });
  card.appendChild(title);

  const desc = document.createElement("p");
  desc.className = "refused-notice";
  desc.textContent = t("card-refused-notice");
  card.appendChild(desc);

  if (reason) {
    const reasonEl = document.createElement("p");
    reasonEl.className = "refused-reason";
    reasonEl.textContent = t("card-refused-reason", { reason });
    card.appendChild(reasonEl);
  }

  return card;
}
