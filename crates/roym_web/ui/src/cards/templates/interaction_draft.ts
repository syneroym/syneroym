export interface InteractionDraftData {
  summary?: string;
  topic?: string;
}

export function renderInteractionDraft(data?: InteractionDraftData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-interaction-draft";
  const title = document.createElement("h3");
  title.textContent = "Interaction Draft";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.summary ? `Summary: ${data.summary}` : "Interaction Draft v1";
  div.appendChild(p);
  return div;
}
