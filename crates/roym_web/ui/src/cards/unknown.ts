export function renderUnknown(type: string, version: number): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-unknown";
  const title = document.createElement("h3");
  title.textContent = `Unknown Card (${type} v${version})`;
  const p = document.createElement("p");
  p.textContent = "This client does not understand this card type or version.";
  div.appendChild(title);
  div.appendChild(p);
  return div;
}
