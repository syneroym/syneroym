/// Renders a fulfilment-receipt card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { FULFILMENT_ACKNOWLEDGED, FULFILMENT_CLAIMED } from "../wording";
import { type AgreedTermsData } from "./quote";

export interface FulfilmentReceiptData {
  agreement?: string;
  conversation?: string;
  consumer_did?: string;
  provider_did?: string;
  role?: "consumer" | "provider";
  terms?: AgreedTermsData;
}

export function renderFulfilmentReceipt(data?: FulfilmentReceiptData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-fulfilment-receipt";

  const title = document.createElement("h3");
  title.textContent = "Fulfilment Receipt";
  card.appendChild(title);

  if (data?.role) {
    const roleP = document.createElement("p");
    roleP.className = "fulfilment-role";
    roleP.textContent = data.role === "provider" ? FULFILMENT_CLAIMED : FULFILMENT_ACKNOWLEDGED;
    card.appendChild(roleP);
  }

  if (data?.terms?.scope) {
    const scopeP = document.createElement("p");
    scopeP.className = "fulfilment-scope";
    scopeP.textContent = `Scope: ${data.terms.scope}`;
    card.appendChild(scopeP);
  }

  return card;
}
