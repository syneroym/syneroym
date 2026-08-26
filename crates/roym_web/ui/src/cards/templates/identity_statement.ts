export interface IdentityStatementData {
  display_name?: string;
  avatar_url?: string;
}

export function renderIdentityStatement(data?: IdentityStatementData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-identity-statement";
  const title = document.createElement("h3");
  title.textContent = "Identity Statement";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent = data?.display_name ? `Display Name: ${data.display_name}` : "Identity Statement v1";
  div.appendChild(p);
  return div;
}
