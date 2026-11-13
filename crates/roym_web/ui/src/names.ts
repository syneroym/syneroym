const SCRIPTS: readonly [string, RegExp][] = [
  ["Latn", /^\p{Script=Latn}$/u],
  ["Deva", /^\p{Script=Deva}$/u],
  ["Taml", /^\p{Script=Taml}$/u],
  ["Beng", /^\p{Script=Beng}$/u],
  ["Gujr", /^\p{Script=Gujr}$/u],
  ["Guru", /^\p{Script=Guru}$/u],
  ["Knda", /^\p{Script=Knda}$/u],
  ["Mlym", /^\p{Script=Mlym}$/u],
  ["Orya", /^\p{Script=Orya}$/u],
  ["Telu", /^\p{Script=Telu}$/u],
  ["Arab", /^\p{Script=Arab}$/u],
  ["Cyrl", /^\p{Script=Cyrl}$/u],
  ["Grek", /^\p{Script=Grek}$/u],
  ["Hani", /^\p{Script=Hani}$/u],
  ["Hira", /^\p{Script=Hira}$/u],
  ["Kana", /^\p{Script=Kana}$/u],
  ["Hang", /^\p{Script=Hang}$/u],
];

const KNOWN_SCRIPTS = [
  "Arab",
  "Beng",
  "Cyrl",
  "Deva",
  "Grek",
  "Gujr",
  "Guru",
  "Hang",
  "Hani",
  "Hira",
  "Kana",
  "Knda",
  "Latn",
  "Mlym",
  "Orya",
  "Taml",
  "Telu",
];

/** The ISO 15924 code of the first letter of `name`, or "Zzzz". */
export function scriptOf(name: string): string {
  const match = name.match(/\p{L}/u);
  if (!match) return "Zzzz";
  const letter = match[0];
  for (const [code, pattern] of SCRIPTS) {
    if (pattern.test(letter)) return code;
  }
  return "Zyyy";
}

function groupRanks(lang: string): Map<string, number> {
  let activeScript = "Latn";
  try {
    const loc = new Intl.Locale(lang).maximize();
    if (loc.script) {
      activeScript = loc.script;
    }
  } catch {
    activeScript = "Latn";
  }

  const remaining = KNOWN_SCRIPTS.filter(
    (s) => s !== activeScript && s !== "Latn" && s !== "Deva" && s !== "Taml",
  );
  remaining.sort();

  const ordered: string[] = [];
  const add = (s: string) => {
    if (!ordered.includes(s)) ordered.push(s);
  };
  add(activeScript);
  add("Latn");
  add("Deva");
  add("Taml");
  for (const s of remaining) {
    add(s);
  }
  add("Zyyy");
  add("Zzzz");

  const map = new Map<string, number>();
  ordered.forEach((s, i) => map.set(s, i));
  return map;
}

const collatorCache = new Map<string, Intl.Collator>();

function getCollator(lang: string): Intl.Collator {
  let coll = collatorCache.get(lang);
  if (!coll) {
    try {
      coll = new Intl.Collator(lang, { numeric: true });
    } catch {
      coll = new Intl.Collator("en", { numeric: true });
    }
    collatorCache.set(lang, coll);
  }
  return coll;
}

/** A copy of `items`, sorted by name for `lang`, grouped by script. */
export function sortByName<T>(
  items: readonly T[],
  nameOf: (item: T) => string,
  lang: string,
): T[] {
  if (items.length <= 1) return [...items];
  const collator = getCollator(lang);
  const ranks = groupRanks(lang);
  const zyyyRank = ranks.get("Zyyy")!;

  const indexed = items.map((item, index) => {
    const name = nameOf(item);
    const script = scriptOf(name);
    const rank = ranks.get(script) ?? zyyyRank;
    return { item, name, rank, index };
  });

  indexed.sort((a, b) => {
    if (a.rank !== b.rank) {
      return a.rank - b.rank;
    }
    const cmp = collator.compare(a.name, b.name);
    if (cmp !== 0) {
      return cmp;
    }
    return a.index - b.index;
  });

  return indexed.map((entry) => entry.item);
}
