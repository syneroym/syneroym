export function renderLink(url: string): HTMLElement {
  const a = document.createElement("a");
  a.href = url;
  a.rel = "noopener noreferrer";
  a.textContent = url;
  return a;
}
