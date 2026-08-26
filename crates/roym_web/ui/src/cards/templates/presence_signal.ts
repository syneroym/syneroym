export interface PresenceSignalData {
  status?: string;
}

export function renderPresenceSignal(data?: PresenceSignalData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-presence-signal";
  const title = document.createElement("h3");
  title.textContent = "Presence Signal";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.status ? `Status: ${data.status}` : "Presence Signal v1";
  div.appendChild(p);
  return div;
}
