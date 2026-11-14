import { t } from "../i18n/i18n";
import { langAttr, languageName, readerLanguages, sameLanguage } from "../i18n/text_lang";
import {
  type Engine,
  keepShown,
  keptTranslation,
  machineNote,
  targetFor,
  translateTexts,
  translationFailure,
  wasShown,
} from "../translation/translation";

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
/// more than one version, adds a note and a button that switches to the next
/// version, in order: the original, the author's translations, the machine
/// version. When the node can translate (`texts` names the strings to send)
/// and the record is in no language the reader reads, adds a button that
/// makes one machine version; a record has at most one. The machine version
/// is always made from the original, never from a translation of it. A
/// machine version this page already made for the same text is drawn
/// again without a second call. It is on screen only if the person had it
/// on screen on this card: `cardKey` names the card, and a card with no key
/// always opens in its best version.
///
/// `host` records what is on screen in `data-shown-lang` (the version's
/// language) and `data-shown-via` (`original`, `author` or `machine`), so a
/// button that signs something can say what the person read.
export function recordText<F>(
  host: HTMLElement,
  versions: TextVersion<F>[],
  draw: (v: TextVersion<F>) => Node[],
  texts?: TranslatableText<F>,
  cardKey?: string,
): void {
  if (versions.length === 0) return;
  const all = [...versions];
  const target = texts ? targetFor(all.map((v) => v.lang)) : null;
  // What the engine is given: the text that binds, in its own language.
  const machine =
    texts && target !== null
      ? { texts, target, sent: texts.get(all[0].fields), sourceLang: all[0].lang || null }
      : null;

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
  let machineIdx: number | null = null;
  let failure = "";

  // Adds the machine version and returns its index.
  const addMachine = (m: NonNullable<typeof machine>, translated: string[], engine: Engine): number => {
    all.push({
      lang: m.target,
      fields: m.texts.set(all[0].fields, translated),
      byAuthor: false,
      machine: engine,
    });
    return all.length - 1;
  };
  const earlier = machine && keptTranslation(machine.sent, machine.sourceLang, machine.target);
  if (machine && earlier) {
    machineIdx = addMachine(machine, earlier.texts, earlier.engine);
    if (cardKey !== undefined && wasShown(cardKey, machine.sent, machine.sourceLang, machine.target)) {
      currentIdx = machineIdx;
    }
  }

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
    if (machine && cardKey !== undefined && machineIdx !== null) {
      keepShown(cardKey, machine.sent, machine.sourceLang, machine.target, currentIdx === machineIdx);
    }
    if (v.machine) {
      noteSpan.textContent = machineNote(v.lang, v.machine);
    } else if (currentIdx !== 0) {
      noteSpan.textContent = t("record-text-by-author", { language: originalLangName });
    } else {
      noteSpan.textContent = t("record-text-original", { language: originalLangName });
    }

    const parts: HTMLElement[] = [noteSpan];
    if (all.length > 1) {
      switchBtn.textContent = switchLabel((currentIdx + 1) % all.length);
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
    currentIdx = (currentIdx + 1) % all.length;
    renderCurrent();
  };

  let translateBtn: HTMLButtonElement | null = null;
  if (machine) {
    translateBtn = document.createElement("button");
    translateBtn.type = "button";
    translateBtn.className = "button record-text-translate";
    translateBtn.textContent = t("translation-translate-button", {
      language: languageName(machine.target),
    });
    const button = translateBtn;
    button.onclick = async () => {
      button.disabled = true;
      try {
        const out = await translateTexts(machine.sent, machine.sourceLang, machine.target);
        machineIdx = addMachine(machine, out.texts, out.engine);
        currentIdx = machineIdx;
        failure = "";
      } catch (err) {
        failure = translationFailure(err);
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
