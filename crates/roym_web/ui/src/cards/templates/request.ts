export interface RequestData {
  summary?: string;
}

export function renderRequest(data?: RequestData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-request";
  const title = document.createElement("h3");
  title.textContent = "Request Card";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.summary ? `Summary: ${data.summary}` : "Request Card v1";
  div.appendChild(p);
  return div;
}
