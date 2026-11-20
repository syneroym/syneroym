/// A commitment's `details`, drawn under the commitment's line on a card.

import { detailsSchema, labelIds, type ExtensionRef } from "../extensions/registry";
import { t } from "../i18n/i18n";
import { fieldView } from "./fields";

interface DetailsData {
  schema?: unknown;
  data?: unknown;
  extra?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function extraList(value: unknown): { label: string; value: string }[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter(isRecord)
    .map((item) => ({ label: String(item.label ?? ""), value: String(item.value ?? "") }));
}

/// `undefined` when the commitment has no details to draw. `lang` is the
/// original language of the record: `details` has no author translations.
export function commitmentDetailsView(
  commitment: Record<string, unknown>,
  currency: string | undefined,
  lang: string,
): HTMLElement | undefined {
  const ext = commitment.extension as ExtensionRef | undefined;
  const details = commitment.details as DetailsData | null | undefined;
  if (!isRecord(ext) || !isRecord(details)) return undefined;
  const schema = typeof details.schema === "string" ? details.schema : "";

  const view = fieldView({
    ext,
    schema: detailsSchema(ext, schema),
    data: isRecord(details.data) ? details.data : {},
    extra: extraList(details.extra),
    currency,
    lang,
    labelIds: labelIds(ext, schema),
  });
  if (view.childElementCount === 0) return undefined;

  const wrap = document.createElement("div");
  wrap.className = `quote-details quote-details-${String(commitment.commitment_id ?? "")}`;
  const heading = document.createElement("p");
  heading.className = "quote-details-heading";
  heading.textContent = t("ext-details-heading");
  wrap.append(heading, view);
  return wrap;
}
