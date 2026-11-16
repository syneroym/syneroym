/// Renders a fulfilment-receipt card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { labelled } from "../../dom";
import { t } from "../../i18n/i18n";
import { recordValue } from "../../records/record_text";
import { type AgreedTermsData } from "./quote";

export interface FulfilmentReceiptData {
  agreement?: string;
  conversation?: string;
  commitment_id?: string;
  commitment_description?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  role?: "giver" | "receiver" | "counterparty" | "quote-author";
  terms?: AgreedTermsData;
  settlement_note?: string | { lang?: string; text?: string };
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
    roleP.textContent =
      data.role === "giver" || data.role === "quote-author"
        ? t("fulfilment-claimed")
        : t("fulfilment-acknowledged");
    card.appendChild(roleP);
  }

  if (data?.commitment_id) {
    const comP = document.createElement("p");
    comP.className = "fulfilment-commitment";
    comP.textContent = data.commitment_description
      ? `${data.commitment_id}: ${data.commitment_description}`
      : data.commitment_id;
    card.appendChild(comP);
  }

  const noteText =
    typeof data?.settlement_note === "object" && data?.settlement_note !== null
      ? (data.settlement_note as { text?: string }).text
      : data?.settlement_note;
  if (noteText) {
    const noteP = document.createElement("p");
    noteP.className = "fulfilment-note";
    noteP.textContent = noteText;
    card.appendChild(noteP);
  }

  const scopeText =
    typeof data?.terms?.scope === "object" && data?.terms?.scope !== null
      ? (data.terms.scope as { text?: string }).text
      : data?.terms?.scope;
  const scopeLang =
    typeof data?.terms?.scope === "object" && data?.terms?.scope !== null
      ? (data.terms.scope as { lang?: string }).lang ?? data.terms.lang ?? ""
      : data?.terms?.lang ?? "";
  if (scopeText) {
    const scopeP = document.createElement("p");
    scopeP.className = "fulfilment-scope";
    labelled(scopeP, t("terms-scope-label"), recordValue(scopeLang, scopeText));
    card.appendChild(scopeP);
  }

  return card;
}
