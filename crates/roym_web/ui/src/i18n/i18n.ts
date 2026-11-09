import { FluentBundle, FluentResource } from "@fluent/bundle";
import EN_BASE_SOURCE from "./en.ftl?raw";
import EN_TAXONOMY_SOURCE from "./taxonomy/en.ftl?raw";

import { sameLanguage } from "./text_lang";

export const FALLBACK_LANGUAGE = "en";
/// Region added to a catalog tag that has none, so money and numbers get
/// Indian digit grouping in every language.
export const FORMAT_REGION = "IN";
const STORAGE_KEY = "roym-hub-language";

type Args = Record<string, string | number>;
/// Catalog tag -> loader of that language's `.ftl` text.
export type Loaders = Record<string, () => Promise<string>>;

const TAXONOMY_LOADERS = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(["./taxonomy/*.ftl", "!./taxonomy/en.ftl"], {
      query: "?raw",
      import: "default",
    }),
  ).map(([path, load]) => [path.slice("./taxonomy/".length, -".ftl".length), load]),
);

const STRING_LOADERS = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(["./*.ftl", "!./en.ftl"], { query: "?raw", import: "default" }),
  ).map(([path, load]) => [path.slice("./".length, -".ftl".length), load]),
);

/// Every `.ftl` file but `en.ftl` is its own script file, loaded only when
/// that language is chosen. A new file here is a new language.
const LOADERS: Loaders = Object.fromEntries(
  Object.entries(STRING_LOADERS).map(([tag, loadCatalog]) => [
    tag,
    async () => {
      const loadTaxonomy = TAXONOMY_LOADERS[tag];
      const [catalog, taxonomy] = await Promise.all([
        loadCatalog(),
        loadTaxonomy ? loadTaxonomy() : Promise.resolve(""),
      ]);
      return taxonomy ? `${catalog}\n${taxonomy}` : catalog;
    },
  ]),
);

interface State {
  language: string;
  locale: string;
  active: FluentBundle;
  english: FluentBundle;
}

/// `IN` is added only when the tag has no region of its own, and digits are
/// always Latin.
function localeFor(tag: string): string {
  const parsed = new Intl.Locale(tag);
  return new Intl.Locale(tag, {
    region: parsed.region ?? FORMAT_REGION,
    numberingSystem: "latn",
  }).toString();
}

/// Fluent's bidi isolation marks are off: the Hub has no right-to-left
/// language yet, and the marks would sit in every copied amount.
function bundleFor(locale: string, source: string): FluentBundle {
  const bundle = new FluentBundle([locale], { useIsolating: false });
  bundle.addResource(new FluentResource(source));
  return bundle;
}

// English exists from the moment this module loads, so `t()` works in unit
// tests and before `initLanguage()` has finished.
const EN_SOURCE = EN_BASE_SOURCE + "\n" + EN_TAXONOMY_SOURCE;
const START_LOCALE = localeFor(FALLBACK_LANGUAGE);
const startEnglish = bundleFor(START_LOCALE, EN_SOURCE);
let state: State = {
  language: FALLBACK_LANGUAGE,
  locale: START_LOCALE,
  active: startEnglish,
  english: startEnglish,
};

export function availableLanguages(loaders: Loaders = LOADERS): string[] {
  return [FALLBACK_LANGUAGE, ...Object.keys(loaders).sort()];
}

export function pickLanguage(
  stored: string | null,
  preferred: readonly string[],
  available: readonly string[],
): string {
  if (stored !== null && available.includes(stored)) return stored;
  for (const tag of preferred) {
    const hit = available.find((candidate) => sameLanguage(tag, candidate));
    if (hit !== undefined) return hit;
  }
  return FALLBACK_LANGUAGE;
}

/// Loads and parses the language first; only a success changes the state
/// and `<html lang>`, so `lang` never says `hi` over English text.
export async function activateWith(tag: string, loaders: Loaders): Promise<void> {
  const locale = localeFor(tag);
  const english = bundleFor(locale, EN_SOURCE);
  let active = english;
  if (tag !== FALLBACK_LANGUAGE) {
    const load = loaders[tag];
    if (load === undefined) throw new Error(`no string catalog for "${tag}"`);
    active = bundleFor(locale, await load());
  }
  state = { language: tag, locale, active, english };
  document.documentElement.lang = tag;
}

export function activate(tag: string): Promise<void> {
  return activateWith(tag, LOADERS);
}

function readStored(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

/// Never rejects: when the chosen catalog cannot load, the Hub stays in
/// English and still draws. The stored choice is kept for the next load.
export async function initLanguageWith(
  loaders: Loaders,
  preferred: readonly string[],
): Promise<void> {
  const tag = pickLanguage(readStored(), preferred, availableLanguages(loaders));
  try {
    await activateWith(tag, loaders);
  } catch (error) {
    console.error(`could not load the "${tag}" string catalog`, error);
  }
}

export function initLanguage(): Promise<void> {
  return initLanguageWith(LOADERS, navigator.languages);
}

export function currentLanguage(): string {
  return state.language;
}

/// The BCP 47 tag `Intl` gets for numbers, money and dates.
export function formatLocale(): string {
  return state.locale;
}

function format(bundle: FluentBundle, id: string, args?: Args): string | undefined {
  const message = bundle.getMessage(id);
  if (message?.value == null) return undefined;
  const errors: Error[] = [];
  const text = bundle.formatPattern(message.value, args, errors);
  if (errors.length > 0) console.error(`could not format message "${id}"`, errors);
  return text;
}

/// The text of message `id` in the Hub language, else in English. Never
/// throws. Call it only when the Hub draws, never when a module loads,
/// or the text would stay English after a switch.
export function t(id: string, args?: Args): string {
  if (typeof id !== "string") {
    console.error("message id is not a string", id);
    return String(id);
  }
  const text = format(state.active, id, args) ?? format(state.english, id, args);
  if (text !== undefined) return text;
  console.error(`missing message "${id}"`);
  return id;
}

export function tEnglish(id: string, args?: Args): string {
  if (typeof id !== "string") {
    return String(id);
  }
  const text = format(state.english, id, args);
  if (text !== undefined) return text;
  return id;
}

export function hasMessage(id: string): boolean {
  return state.english.hasMessage(id);
}

/// Marks an id that is kept in a table. Read such a table with
/// `lookupText`, so every `t()` call outside this file has a literal id.
export function messageId(id: string): string {
  return id;
}

/// `undefined` for a key the table does not know, so the caller can show
/// the value as it came.
export function lookupText<K>(table: Map<K, string>, key: K, args?: Args): string | undefined {
  const id = table.get(key);
  return id === undefined ? undefined : t(id, args);
}

/// The text for a code, or the code as it came when the table lacks it:
/// a code from another node is never hidden and never guessed. Every screen
/// that shows a code as a word reads its table through this function. The
/// one other reader is `errText`, whose unknown code has a sentence of its own.
export function codeWord(table: Map<string, string>, code: string): string {
  return lookupText(table, code) ?? code;
}

/// True when the choice was written and reads back.
export function saveLanguage(tag: string): boolean {
  try {
    localStorage.setItem(STORAGE_KEY, tag);
    return localStorage.getItem(STORAGE_KEY) === tag;
  } catch {
    return false;
  }
}
