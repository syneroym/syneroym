export interface EventLogEntryData {
  event?: string;
  timestamp?: number;
}

export function renderEventLogEntry(data?: EventLogEntryData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-event-log-entry";
  const title = document.createElement("h3");
  title.textContent = "Event Log Entry";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.event ? `Event: ${data.event}` : "Event Log Entry v1";
  div.appendChild(p);
  return div;
}
