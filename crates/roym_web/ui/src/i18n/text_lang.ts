import { currentLanguage } from "./i18n";

/// The languages a person can pick for a record, in this order.
export const RECORD_LANGUAGES: readonly string[] = [
  "en", "hi", "hi-Latn", "bn", "gu", "kn", "ml", "mr", "or", "pa", "ta", "ta-Latn", "te",
];

/// Same language and same script, so hi-Latn does not match the Devanagari
/// hi catalog. A tag `Intl.Locale` refuses matches nothing.
export function sameLanguage(requested: string, available: string): boolean {
  try {
    const a = new Intl.Locale(requested);
    const b = new Intl.Locale(available);
    return a.language === b.language && a.maximize().script === b.maximize().script;
  } catch {
    return false;
  }
}

/// The name of `tag` in the language `inLanguage` (default: the UI language),
/// or the tag itself when `Intl` has no name or refuses the tag.
export function languageName(tag: string, inLanguage?: string): string {
  const target = inLanguage ?? currentLanguage();
  try {
    return new Intl.DisplayNames([target], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/// The value for an element's `lang`: the tag, or "" when `Intl.Locale` refuses it.
export function langAttr(tag: string): string {
  try {
    new Intl.Locale(tag);
    return tag;
  } catch {
    return "";
  }
}

/// A `<select>` of RECORD_LANGUAGES (plus `current` when it is not in the
/// list), each option named by languageName, starting at `current`.
export function textLangSelect(className: string, current: string): HTMLSelectElement {
  const select = document.createElement("select");
  select.className = className;
  const list = RECORD_LANGUAGES.includes(current)
    ? [...RECORD_LANGUAGES]
    : [...RECORD_LANGUAGES, current];
  for (const tag of list) {
    const option = document.createElement("option");
    option.value = tag;
    option.textContent = languageName(tag);
    select.appendChild(option);
  }
  select.value = current;
  return select;
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
