export interface CatalogItemData {
  title?: string;
  description?: string;
  price?: string;
}

export function renderCatalogItem(data?: CatalogItemData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-catalog-item";
  const title = document.createElement("h3");
  title.textContent = "Catalog Item";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.title ? `Title: ${data.title}` : "Catalog Item v1";
  div.appendChild(p);
  return div;
}
