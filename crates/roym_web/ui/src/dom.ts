import { RpcError } from "./rpc";

/// A stranger's directory label, listing text or error string is only ever
/// a text node -- never markup, never a URL turned into a link.
export function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

export function errText(err: unknown): string {
  if (err instanceof RpcError) return err.message;
  return err instanceof Error ? err.message : String(err);
}

export function field(labelText: string, control: HTMLElement): HTMLElement {
  const label = document.createElement("label");
  label.className = "field";
  label.appendChild(text("span", labelText));
  label.appendChild(control);
  return label;
}
