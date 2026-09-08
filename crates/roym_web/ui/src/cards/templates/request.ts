/// Renders a request card.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no insertAdjacentHTML,
///   no style from data, no attributes from data other than classes.
/// - No URL is fetched, prefetched, or navigated to.
/// - A missing optional field renders as an omitted row, never as undefined
///   and never as a guess.

export interface RequestData {
  request_id?: string;
  conversation?: string;
  sequence?: number;
  listing_id?: string;
  categories?: string[];
  description?: string;
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
  title.textContent = "Request";
  card.appendChild(title);

  const desc = data?.description ?? data?.summary;
  if (desc !== undefined && desc !== null && desc !== "") {
    const descP = document.createElement("p");
    descP.className = "request-description";
    descP.textContent = desc;
    card.appendChild(descP);
  }

  if (data?.categories && data.categories.length > 0) {
    const catsP = document.createElement("p");
    catsP.className = "request-categories";
    catsP.textContent = `Categories: ${data.categories.join(", ")}`;
    card.appendChild(catsP);
  }

  if (data?.listing_id) {
    const listP = document.createElement("p");
    listP.className = "request-listing";
    listP.textContent = `Listing: ${data.listing_id}`;
    card.appendChild(listP);
  }

  if (data?.window) {
    const winP = document.createElement("p");
    winP.className = "request-window";
    const start = new Date(data.window.earliest_secs * 1000).toISOString();
    const end = new Date(data.window.latest_secs * 1000).toISOString();
    winP.textContent = `Window: ${start} - ${end}`;
    card.appendChild(winP);
  }

  if (data?.data_use_notice) {
    const noticeP = document.createElement("p");
    noticeP.className = "request-data-use-notice";
    noticeP.textContent = `Notice: ${data.data_use_notice}`;
    card.appendChild(noticeP);
  }

  return card;
}
