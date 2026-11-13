import { HubError } from "./hub-error";
import { codeWord, lookupText, messageId, t } from "./i18n/i18n";
import { RpcError } from "./rpc";

/// A stranger's directory label, listing text or error string is only ever
/// a text node -- never markup, never a URL turned into a link.
export function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

/// Message id for each JSON-RPC code a Roym service or the `web` proxy
/// answers with. Any other code uses `error-service`. A `Map`, so a code
/// from another node can never hit an inherited property.
const ERROR_MESSAGES = new Map<number, string>([
  [-32001, messageId("error-service-unavailable")],
  [-32002, messageId("error-service-timeout")],
  [-32003, messageId("error-permission")],
  [-32004, messageId("error-quota")],
  [-32020, messageId("error-translation")],
  [-32010, messageId("error-not-signed-in")],
  [-32011, messageId("error-not-owner")],
  [-32012, messageId("error-no-owner")],
  [-32013, messageId("error-not-local")],
  [-32601, messageId("error-method-not-found")],
  [-32602, messageId("error-invalid-input")],
  [-32603, messageId("error-internal")],
]);

function rpcErrorText(err: RpcError): string {
  // Codes go in as strings: a number argument would print as `-32,602`.
  const args = { detail: err.message, code: String(err.code) };
  if (err.code > 0) return t("error-http", args);
  if (err.code === 0) return t("error-network", args);
  return lookupText(ERROR_MESSAGES, err.code, args) ?? t("error-service", args);
}

/// The text to show for a failure, in the Hub language. The service's own
/// English text is only a `$detail` inside the Hub's message. A `HubError`
/// is shown as it is; any other failure (a browser error, a thrown string)
/// is a `$detail` inside the Hub's own sentence.
export function errText(err: unknown): string {
  if (err instanceof RpcError) return rpcErrorText(err);
  if (err instanceof HubError) return err.message;
  const detail = err instanceof Error ? err.message : String(err);
  return t("error-unexpected", { detail });
}

/// A label, one space, then the value nodes. Fluent removes a space at the
/// end of a message, so the space between a label and its value is added
/// here and not in the string catalog. `label` is the translated text, so
/// each `t()` call keeps a literal id at the call site.
export function labelled(parent: HTMLElement, label: string, ...nodes: (Node | string)[]): void {
  const span = document.createElement("span");
  span.textContent = label;
  parent.append(span, " ", ...nodes);
}

export function field(labelText: string, control: HTMLElement): HTMLElement {
  const label = document.createElement("label");
  label.className = "field";
  label.appendChild(text("span", labelText));
  label.appendChild(control);
  return label;
}

export function input(placeholder: string, value = ""): HTMLInputElement {
  const el = document.createElement("input");
  el.placeholder = placeholder;
  el.value = value;
  return el;
}

/// A `<select>` of `[value, label]` pairs. The value is the code; the label
/// is the text the person reads.
export function select(options: Array<[string, string]>, value?: string): HTMLSelectElement {
  const el = document.createElement("select");
  for (const [optValue, optLabel] of options) {
    const o = document.createElement("option");
    o.value = optValue;
    o.textContent = optLabel;
    el.appendChild(o);
  }
  if (value) el.value = value;
  return el;
}

/// The options for `codes`, each shown as the word of `table`.
export function codeOptions(table: Map<string, string>, codes: string[]): Array<[string, string]> {
  return codes.map((code): [string, string] => [code, codeWord(table, code)]);
}

export function checkbox(labelText: string, checked = false): { wrap: HTMLElement; box: HTMLInputElement } {
  const wrap = document.createElement("label");
  wrap.className = "checkbox-field";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  wrap.append(box, text("span", labelText));
  return { wrap, box };
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
