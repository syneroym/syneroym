import { t } from "../i18n/i18n";

export function renderUnknown(type: string, version: number): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-unknown";
  const title = document.createElement("h3");
  title.textContent = t("card-unknown-title", { type, version: String(version) });
  const p = document.createElement("p");
  p.textContent = t("card-unknown-body");
  div.appendChild(title);
  div.appendChild(p);
  return div;
}
