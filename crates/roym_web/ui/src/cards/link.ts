const ALLOWED_SCHEMES = new Set(["https:", "http:"]);

export function renderLink(url: string): Node {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return document.createTextNode(url);
  }
  if (!ALLOWED_SCHEMES.has(parsed.protocol)) {
    return document.createTextNode(url);
  }
  const a = document.createElement("a");
  a.href = parsed.href;
  a.rel = "noopener noreferrer";
  a.textContent = url;
  return a;
}
