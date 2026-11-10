import { currentLanguage, t } from "../i18n/i18n";
import { langAttr, languageName, sameLanguage } from "../i18n/text_lang";

export interface TextVersion<F> {
  lang: string;
  fields: F;
  byAuthor: boolean;
}

/// The UI language, then navigator.languages.
export function readerLanguages(): string[] {
  const ui = currentLanguage();
  const nav: readonly string[] =
    typeof navigator !== "undefined" && Array.isArray(navigator.languages)
      ? navigator.languages
      : [];
  return Array.from(new Set([ui, ...nav]));
}

/// Index into `versions` (0 is the original): for each reader language in
/// order, the original if it matches, else the first translation that
/// matches; else 0.
export function bestVersion<F>(versions: TextVersion<F>[], reader: readonly string[]): number {
  if (versions.length === 0) return 0;

  for (const tag of reader) {
    if (sameLanguage(tag, versions[0].lang)) {
      return 0;
    }
    for (let i = 1; i < versions.length; i++) {
      if (sameLanguage(tag, versions[i].lang)) {
        return i;
      }
    }
  }

  return 0;
}

/// Draws `draw(version)` for the best version into `host`. When there is
/// more than one version, adds a note and a button that switches between
/// the shown version and the original.
export function recordText<F>(
  host: HTMLElement,
  versions: TextVersion<F>[],
  draw: (v: TextVersion<F>) => Node[],
): void {
  if (versions.length === 0) return;
  if (versions.length === 1) {
    for (const node of draw(versions[0])) {
      host.appendChild(node);
    }
    return;
  }

  const initialIdx = bestVersion(versions, readerLanguages());
  let currentIdx = initialIdx;
  const alternateIdx = initialIdx === 0 ? 1 : 0;

  const contentBox = document.createElement("div");
  contentBox.className = "record-text-content";

  const noteBox = document.createElement("div");
  noteBox.className = "record-text-meta";

  const noteSpan = document.createElement("span");
  noteSpan.className = "record-text-note";

  const switchBtn = document.createElement("button");
  switchBtn.type = "button";
  switchBtn.className = "button record-text-switch";

  const originalLangName = languageName(versions[0].lang);

  function renderCurrent() {
    contentBox.replaceChildren(...draw(versions[currentIdx]));
    if (currentIdx !== 0) {
      noteSpan.textContent = t("record-text-by-author", { language: originalLangName });
      switchBtn.textContent = t("record-text-show-original");
    } else {
      noteSpan.textContent = t("record-text-original", { language: originalLangName });
      switchBtn.textContent = t("record-text-show-translation");
    }
  }

  switchBtn.onclick = () => {
    currentIdx = currentIdx === initialIdx ? alternateIdx : initialIdx;
    renderCurrent();
  };

  renderCurrent();
  noteBox.append(noteSpan, switchBtn);
  host.append(contentBox, noteBox);
}

/// Creates a span with `lang` set to `langAttr(tag)` (unconditionally, so refused tags get lang="").
/// Inserts `value` as text or child node.
export function recordValue(tag: string, value: Node | string): HTMLSpanElement {
  const span = document.createElement("span");
  span.setAttribute("lang", langAttr(tag));
  if (typeof value === "string") {
    span.textContent = value;
  } else {
    span.appendChild(value);
  }
  return span;
}
