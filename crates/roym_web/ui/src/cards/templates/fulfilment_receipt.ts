/// Renders a fulfilment-receipt card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { t } from "../../i18n/i18n";
import { type AgreedTermsData } from "./quote";

export interface FulfilmentReceiptData {
  agreement?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  role?: "counterparty" | "quote-author";
  terms?: AgreedTermsData;
}

export function renderFulfilmentReceipt(data?: FulfilmentReceiptData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-fulfilment-receipt";

  const title = document.createElement("h3");
  title.textContent = t("fulfilment-title");
  card.appendChild(title);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "fulfilment-role";
    roleP.textContent = data.role === "quote-author" ? t("fulfilment-claimed") : t("fulfilment-acknowledged");
    card.appendChild(roleP);
  }

  if (data?.terms?.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "fulfilment-scope";
    scopeP.textContent = t("terms-scope", { value: data.terms.scope });
    card.appendChild(scopeP);
  }

  return card;
}
