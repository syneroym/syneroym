export interface IntentBeaconData {
  intent?: string;
  scope?: string;
}

export function renderIntentBeacon(data?: IntentBeaconData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-intent-beacon";
  const title = document.createElement("h3");
  title.textContent = "Intent Beacon";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.intent ? `Intent: ${data.intent}` : "Intent Beacon v1";
  div.appendChild(p);
  return div;
}
