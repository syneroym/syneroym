/// Renders a step card: one work step that a party recorded for a commitment.
///
/// Rules:
/// - Every value is textContent. No innerHTML, no style from data, no
///   attribute from data other than `lang`.
/// - No URL is fetched, prefetched, or navigated to.
/// - A step of an extension this build does not know is drawn as it came,
///   under its own type, with the "not checked" line. It is never hidden and
///   never guessed.

import { stepNameId, stepType, type ExtensionRef } from "../../extensions/registry";
import { codeWord, t } from "../../i18n/i18n";
import { recordValue } from "../../records/record_text";
import { fieldView } from "../fields";

export interface StepData {
  agreement?: string;
  conversation?: string;
  commitment_id?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  role?: "giver" | "receiver";
  extension?: ExtensionRef;
  step_type?: string;
  data?: Record<string, unknown>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function renderStep(data?: StepData): HTMLElement {
  const card = document.createElement("div");
  card.className = "card card-step";

  const ext = isRecord(data?.extension) ? data.extension : undefined;
  const type = typeof data?.step_type === "string" ? data.step_type : "";
  const known = ext && type ? stepType(ext, type) : undefined;

  const title = document.createElement("h3");
  if (known && ext) {
    // Read through `codeWord`: the id is derived from the manifest, not written
    // here, and a name missing from the catalog shows as the type.
    title.textContent = codeWord(new Map([[type, stepNameId(ext.id, type)]]), type);
  } else {
    title.append(recordValue("", type));
  }
  card.appendChild(title);

  if (data?.role === "giver" || data?.role === "receiver") {
    const roleP = document.createElement("p");
    roleP.className = "step-role";
    roleP.textContent = data.role === "giver" ? t("role-giver") : t("role-receiver");
    card.appendChild(roleP);
  }

  if (data?.commitment_id) {
    const commitmentP = document.createElement("p");
    commitmentP.className = "step-commitment";
    commitmentP.append(recordValue("", data.commitment_id));
    card.appendChild(commitmentP);
  }

  if (ext) {
    const view = fieldView({
      ext,
      schema: known ? { schema: type, fields: known.fields } : undefined,
      data: isRecord(data?.data) ? data.data : {},
      labelIds: new Map(),
    });
    if (view.childElementCount > 0) card.appendChild(view);
  }
  return card;
}
