import { errText } from "../dom";
import { t } from "../i18n/i18n";
import { langAttr, languageName, readerLanguages, sameLanguage } from "../i18n/text_lang";
import { type Engine, machineNote, targetFor, translateTexts } from "../translation/translation";

export interface TextVersion<F> {
  lang: string;
  fields: F;
  byAuthor: boolean;
  /// Set on a machine translation made on this device. It is for display
  /// only: it is never signed, and the original is what binds.
  machine?: Engine;
}

/// The translatable strings of a record's fields in a fixed order, and the
/// way back from the translated strings to the fields.
export interface TranslatableText<F> {
  get: (fields: F) => string[];
  set: (fields: F, texts: string[]) => F;
}

function isLink(value: string): boolean {
  return value.startsWith("http://") || value.startsWith("https://");
}

/// The `TranslatableText` for the named text fields of `F`. A field whose
/// value is a link is left out of `get` and kept as it is by `set`: the
/// engine never sees a link, so a machine version cannot show a link the
/// author did not sign. An empty or absent field is sent as "".
export function textFields<F extends object>(keys: readonly (keyof F & string)[]): TranslatableText<F> {
  const sent = (fields: F) =>
    keys.filter((key) => !isLink(String((fields as Record<string, unknown>)[key] ?? "")));
  return {
    get: (fields) => sent(fields).map((key) => String((fields as Record<string, unknown>)[key] ?? "")),
    set: (fields, texts) => {
      const out: Record<string, unknown> = { ...(fields as Record<string, unknown>) };
      sent(fields).forEach((key, i) => {
        out[key] = texts[i];
      });
      return out as F;
    },
  };
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
/// the original and the other version the person looked at last. When the
/// node can translate (`texts` names the strings to send) and the record is
/// in no language the reader reads, adds a button that makes one machine
/// version; a record has at most one.
///
/// `host` records what is on screen in `data-shown-lang` (the version's
/// language) and `data-shown-via` (`original`, `author` or `machine`), so a
/// button that signs something can say what the person read.
export function recordText<F>(
  host: HTMLElement,
  versions: TextVersion<F>[],
  draw: (v: TextVersion<F>) => Node[],
  texts?: TranslatableText<F>,
): void {
  if (versions.length === 0) return;
  const all = [...versions];
  const target = texts ? targetFor(all.map((v) => v.lang)) : null;

  const markShown = (idx: number) => {
    const v = all[idx];
    host.dataset.shownLang = v.lang;
    host.dataset.shownVia = v.machine ? "machine" : idx === 0 ? "original" : "author";
  };

  if (all.length === 1 && target === null) {
    markShown(0);
    for (const node of draw(all[0])) {
      host.appendChild(node);
    }
    return;
  }

  let currentIdx = all.length > 1 ? bestVersion(all, readerLanguages()) : 0;
  // The version the switch button shows when the original is on screen.
  let lastOther: number | null = all.length > 1 ? (currentIdx !== 0 ? currentIdx : 1) : null;
  let machineIdx: number | null = null;
  let failure = "";

  const contentBox = document.createElement("div");
  contentBox.className = "record-text-content";

  const noteBox = document.createElement("div");
  noteBox.className = "record-text-meta";

  const noteSpan = document.createElement("span");
  noteSpan.className = "record-text-note";

  const switchBtn = document.createElement("button");
  switchBtn.type = "button";
  switchBtn.className = "button record-text-switch";

  const originalLangName = languageName(all[0].lang);

  const switchLabel = (next: number): string => {
    if (next === 0) return t("record-text-show-original");
    if (all[next].machine) {
      return t("translation-translate-button", { language: languageName(all[next].lang) });
    }
    return t("record-text-show-translation");
  };

  function renderCurrent() {
    const v = all[currentIdx];
    contentBox.replaceChildren(...draw(v));
    markShown(currentIdx);
    if (v.machine) {
      noteSpan.textContent = machineNote(v.lang, v.machine);
    } else if (currentIdx !== 0) {
      noteSpan.textContent = t("record-text-by-author", { language: originalLangName });
    } else {
      noteSpan.textContent = t("record-text-original", { language: originalLangName });
    }

    const parts: HTMLElement[] = [noteSpan];
    const next = currentIdx === 0 ? lastOther : 0;
    if (next !== null) {
      switchBtn.textContent = switchLabel(next);
      parts.push(switchBtn);
    }
    if (translateBtn && machineIdx === null) parts.push(translateBtn);
    if (failure) {
      const failureEl = document.createElement("p");
      failureEl.className = "record-text-error";
      failureEl.textContent = failure;
      parts.push(failureEl);
    }
    noteBox.replaceChildren(...parts);
  }

  switchBtn.onclick = () => {
    const next = currentIdx === 0 ? lastOther : 0;
    if (next === null) return;
    currentIdx = next;
    if (next !== 0) lastOther = next;
    renderCurrent();
  };

  let translateBtn: HTMLButtonElement | null = null;
  if (texts && target !== null) {
    translateBtn = document.createElement("button");
    translateBtn.type = "button";
    translateBtn.className = "button record-text-translate";
    translateBtn.textContent = t("translation-translate-button", {
      language: languageName(target),
    });
    const button = translateBtn;
    button.onclick = async () => {
      button.disabled = true;
      const shown = all[currentIdx];
      try {
        const out = await translateTexts(texts.get(shown.fields), shown.lang || null, target);
        all.push({
          lang: target,
          fields: texts.set(shown.fields, out.texts),
          byAuthor: false,
          machine: out.engine,
        });
        machineIdx = all.length - 1;
        lastOther = machineIdx;
        currentIdx = machineIdx;
        failure = "";
      } catch (err) {
        failure = t("translation-failed", { detail: errText(err) });
      }
      button.disabled = false;
      renderCurrent();
    };
  }

  renderCurrent();
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
