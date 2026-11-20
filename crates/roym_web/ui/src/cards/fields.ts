/// The generic view of a commitment's `details` or a step's `data`.
///
/// It draws the fields of a schema as labelled values, with the labels from
/// the string catalog. A client that does not know the extension draws the
/// data as it came and says that it was not checked.
///
/// Rules, as for every card template: every value is `textContent`, and no
/// attribute comes from data other than `lang`.

import { labelled } from "../dom";
import { formatDateTime, formatDuration, formatNumber } from "../i18n/format";
import { codeWord, t } from "../i18n/i18n";
import { formatMinor } from "../money";
import { recordValue } from "../records/record_text";
import type { ExtensionRef, FieldDef, SchemaDef } from "../extensions/registry";

export interface FieldViewInput {
  ext: ExtensionRef;
  /// `undefined` for an extension this build does not know.
  schema?: SchemaDef;
  data: Record<string, unknown>;
  extra?: { label: string; value: string }[];
  /// The deal's currency, for `money` fields.
  currency?: string;
  /// The record's original `lang`, for free text.
  lang?: string;
  /// Message ids by field name, and by `field:choice` (see `labelIds`).
  labelIds: Map<string, string>;
}

function line(root: HTMLElement, ...classes: string[]): HTMLParagraphElement {
  const p = document.createElement("p");
  p.classList.add(...classes);
  root.appendChild(p);
  return p;
}

/// A value of another type than its field says is shown as it came, so a
/// value is never hidden.
function plain(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function valueNode(field: FieldDef, value: unknown, input: FieldViewInput): Node {
  switch (field.type) {
    case "text":
    case "long-text":
      return recordValue(input.lang ?? "", plain(value));
    case "number":
      return document.createTextNode(typeof value === "number" ? formatNumber(value) : plain(value));
    case "money":
      if (typeof value !== "number") return document.createTextNode(plain(value));
      return document.createTextNode(input.currency ? formatMinor(value, input.currency) : formatNumber(value));
    case "date-time":
      return document.createTextNode(typeof value === "number" ? formatDateTime(value) : plain(value));
    case "duration":
      return document.createTextNode(typeof value === "number" ? formatDuration(value) : plain(value));
    case "choice":
      return document.createTextNode(codeWord(input.labelIds, `${field.name}:${plain(value)}`));
    case "code":
      return document.createTextNode(plain(value));
  }
}

export function fieldView(input: FieldViewInput): HTMLElement {
  const root = document.createElement("div");
  root.className = "field-view";

  if (input.schema) {
    for (const field of input.schema.fields) {
      if (!Object.hasOwn(input.data, field.name)) continue;
      const p = line(root, "field-view-line", `field-view-${field.type}`);
      const label = codeWord(input.labelIds, field.name);
      labelled(p, label, valueNode(field, input.data[field.name], input));
    }
  } else {
    // The keys and values come from another node, in no language we know.
    for (const key of Object.keys(input.data)) {
      const p = line(root, "field-view-line", "field-view-data");
      p.append(recordValue("", `${key}:`), " ", recordValue("", plain(input.data[key])));
    }
    line(root, "field-view-not-checked").textContent = t("ext-not-checked");
  }

  for (const item of input.extra ?? []) {
    const p = line(root, "field-view-line", "field-view-extra");
    p.append(recordValue(input.lang ?? "", `${item.label}:`), " ", recordValue(input.lang ?? "", item.value));
  }
  return root;
}
