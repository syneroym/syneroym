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

/// Returns a function that refills `host` from the node. The content is
/// built off the page and swapped in only when no newer run has started
/// meanwhile, so two overlapping runs (a double click, or a reload that
/// follows an action) can never leave two copies of the rows. A run that
/// was replaced draws nothing and resolves to `undefined`; otherwise it
/// resolves to whatever `fill` returned.
export function refiller<A extends unknown[], T>(
  host: Element,
  fill: (into: DocumentFragment, ...args: A) => Promise<T>,
): (...args: A) => Promise<T | undefined> {
  let newest = 0;
  return async (...args) => {
    const mine = ++newest;
    const into = document.createDocumentFragment();
    const result = await fill(into, ...args);
    if (mine !== newest) return undefined;
    host.replaceChildren(into);
    return result;
  };
}
