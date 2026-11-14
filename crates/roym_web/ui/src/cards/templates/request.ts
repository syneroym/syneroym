/// Renders a request card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A `<time>` element's `datetime` is computed by the Hub from a number,
///   never copied from a string of the card.
/// - A time that ends a row is a `<time>` element. A time inside a sentence
///   is plain text in the string catalog message.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

import { labelled } from "../../dom";
import { timeElement } from "../../i18n/format";
import { t } from "../../i18n/i18n";
import { langAttr } from "../../i18n/text_lang";
import { recordText, textFields, type TextVersion } from "../../records/record_text";
import { categoryLabel } from "../../taxonomy/taxonomy";

export interface RequestData {
  request_id?: string;
  conversation?: string;
  sequence?: number;
  listing_id?: string;
  categories?: string[];
  description?: string;
  lang?: string;
  translations?: Array<{ lang: string; description: string }>;
  area?: {
    center?: { lat: number; lon: number };
    radius_m?: number;
  };
  window?: {
    earliest_secs: number;
    latest_secs: number;
  };
  data_use_notice?: string;
  summary?: string;
}

export function renderRequest(data?: RequestData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-request";

  const title = document.createElement("h3");
  title.textContent = t("request-title");
  card.appendChild(title);

  const desc = data?.description ?? data?.summary;
  if (desc !== undefined && desc !== null && desc !== "") {
    const versions: TextVersion<{ description: string }>[] = [
      {
        lang: data?.lang ?? "",
        fields: { description: desc },
        byAuthor: false,
      },
      ...(data?.translations || []).map((tr) => ({
        lang: tr.lang,
        fields: { description: tr.description },
        byAuthor: true,
      })),
    ];
    recordText(card, versions, (v) => {
      const descP = document.createElement("p");
      descP.className = "request-description";
      descP.textContent = v.fields.description;
      descP.setAttribute("lang", langAttr(v.lang));
      return [descP];
    }, textFields<{ description: string }>(["description"]),
    data?.request_id !== undefined ? `request:${data.request_id}` : undefined);
  }

  if (data?.categories && data.categories.length > 0) {
    const catsP = document.createElement("p");
    catsP.className = "request-categories";
    const labels = data.categories.map((c) => categoryLabel(c)).join(", ");
    catsP.textContent = t("request-categories", { categories: labels });
    card.appendChild(catsP);
  }

  if (data?.listing_id) {
    const listP = document.createElement("p");
    listP.className = "request-listing";
    listP.textContent = t("request-listing", { id: data.listing_id });
    card.appendChild(listP);
  }

  if (data?.window) {
    const winP = document.createElement("p");
    winP.className = "request-window";
    labelled(
      winP,
      t("request-window-label"),
      timeElement(data.window.earliest_secs),
      t("terms-range-separator"),
      timeElement(data.window.latest_secs),
    );
    card.appendChild(winP);
  }

  if (data?.data_use_notice) {
    const noticeP = document.createElement("p");
    noticeP.className = "request-data-use-notice";
    const valSpan = document.createElement("span");
    valSpan.textContent = data.data_use_notice;
    labelled(noticeP, t("request-notice-label"), valSpan);
    card.appendChild(noticeP);
  }

  return card;
}
