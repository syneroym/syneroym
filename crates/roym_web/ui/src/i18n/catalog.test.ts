import { FluentBundle, FluentResource } from "@fluent/bundle";
import { describe, expect, it } from "vitest";
import { findMessageIds } from "./scan";

const catalogs = import.meta.glob<string>("./*.ftl", {
  query: "?raw",
  import: "default",
  eager: true,
});
const sources = import.meta.glob<string>("../**/*.ts", {
  query: "?raw",
  import: "default",
  eager: true,
});

const ENTRY = /^-?[A-Za-z][A-Za-z0-9_-]*\s*=/gm;
const ID = /^(-?[A-Za-z][A-Za-z0-9_-]*)\s*=/;

const byTag = Object.fromEntries(
  Object.entries(catalogs).map(([path, text]) => [path.slice(2, -".ftl".length), text]),
);
const english = byTag["en"];
const hindi = byTag["hi"];

/// message id -> the variables its text uses. Comment lines are dropped
/// first, so a `$name` in a comment is not counted.
function variablesById(source: string): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  let current: Set<string> | undefined;
  for (const line of source.split("\n")) {
    if (line.startsWith("#")) continue;
    const id = ID.exec(line)?.[1];
    if (id !== undefined) {
      current = new Set();
      out.set(id, current);
    }
    for (const match of line.matchAll(/\$([A-Za-z][A-Za-z0-9_-]*)/g)) current?.add(match[1]);
  }
  return out;
}

describe("the string catalog files", () => {
  it("are named by a canonical language tag", () => {
    for (const name of Object.keys(byTag)) {
      expect(new Intl.Locale(name).toString()).toBe(name);
      expect(name).not.toBe("und");
    }
  });

  it("have no message that Fluent would drop without an error", () => {
    for (const [name, text] of Object.entries(byTag)) {
      const entries = text.match(ENTRY)?.length ?? 0;
      expect(new FluentResource(text).body.length, name).toBe(entries);
    }
  });

  it("have no line that ends with a space or a tab", () => {
    for (const [name, text] of Object.entries(byTag)) {
      expect(text.split("\n").filter((line) => /[ \t]$/.test(line)), name).toEqual([]);
    }
  });

  it("give Hindi exactly the ids of English", () => {
    const ids = (text: string) => [...variablesById(text).keys()].sort();
    expect(ids(hindi)).toEqual(ids(english));
  });

  it("give Hindi only variables that English has", () => {
    const en = variablesById(english);
    for (const [id, vars] of variablesById(hindi)) {
      for (const name of vars) expect(en.get(id), `${id} uses $${name}`).toContain(name);
    }
  });

  it("format every message without an error in each language", () => {
    for (const [name, text] of Object.entries(byTag)) {
      const bundle = new FluentBundle([name], { useIsolating: false });
      bundle.addResource(new FluentResource(text));
      for (const [id, vars] of variablesById(text)) {
        const args = Object.fromEntries([...vars].map((v) => [v, "1"]));
        const errors: Error[] = [];
        bundle.formatPattern(bundle.getMessage(id)!.value!, args, errors);
        expect(errors, `${name}/${id}`).toEqual([]);
      }
    }
  });
});

/// Messages that hold no word by design: they are a separator, a name or a
/// shape that reads the same in every language. Any other Hindi message that
/// has no Devanagari letter is an English sentence that was left in.
const NO_WORDS = [
  // A dash between two values.
  "terms-range-separator",
  // The product name, written the same in every language.
  "tab-synorg",
  // `v` and a version number.
  "listing-version",
  // Two times, a dash and a count.
  "listing-slot-line",
  // Four values joined by punctuation.
  "safety-report-row",
  // The protocol name and a status number; the outer sign-in sentence is Hindi.
  "login-error-http",
];

describe("the Hindi file", () => {
  it("has a Devanagari letter in every message, except the ones that hold no word", () => {
    const bodies = new Map<string, string>();
    let current: string | undefined;
    for (const line of hindi.split("\n")) {
      if (line.startsWith("#")) continue;
      const id = ID.exec(line)?.[1];
      if (id !== undefined) current = id;
      if (current !== undefined) bodies.set(current, `${bodies.get(current) ?? ""}${line}\n`);
    }
    const bare = [...bodies.entries()]
      .filter(([id, body]) => !NO_WORDS.includes(id) && !/[ऀ-ॿ]/.test(body))
      .map(([id]) => id);
    expect(bare).toEqual([]);
  });

  it("lists in NO_WORDS only messages that exist", () => {
    const ids = new Set(variablesById(hindi).keys());
    expect(NO_WORDS.filter((id) => !ids.has(id))).toEqual([]);
  });
});

describe("the ids the source uses", () => {
  const used = Object.entries(sources)
    .filter(([path]) => !path.endsWith(".test.ts") && path !== "./i18n.ts")
    .map(([path, source]) => findMessageIds(source, path));

  it("are all string literals, called as `t` or `messageId`", () => {
    expect(used.flatMap((scan) => scan.problems)).toEqual([]);
  });

  it("are all in the English file", () => {
    const known = variablesById(english);
    const missing = used.flatMap((scan) => scan.ids).filter((id) => !known.has(id));
    expect(missing).toEqual([]);
  });
});

describe("the English file", () => {
  /// Held in the Rust crate too and checked against this file there, but
  /// no Hub screen shows it: the service sends it as the error text.
  const KEPT_FOR_RUST = ["group-add-unreachable"];

  it("has no message that nothing uses", () => {
    const used = new Set(
      Object.entries(sources)
        .filter(([path]) => !path.endsWith(".test.ts"))
        .flatMap(([path, source]) => findMessageIds(source, path).ids),
    );
    const unused = [...variablesById(english).keys()].filter(
      (id) => !used.has(id) && !KEPT_FOR_RUST.includes(id),
    );
    expect(unused).toEqual([]);
  });
});

describe("findMessageIds", () => {
  it("ignores a call inside a comment", () => {
    expect(findMessageIds('// t("x")\n/* t("y") */').ids).toEqual([]);
  });

  it("ignores other functions", () => {
    expect(findMessageIds('format("x"); list("y"); a.t("z");').ids).toEqual([]);
  });

  it("reports an id that is not a literal", () => {
    const scan = findMessageIds("t(`a-${b}`); t(id);");
    expect(scan.ids).toEqual([]);
    expect(scan.problems).toHaveLength(2);
  });

  it("finds an id given to messageId", () => {
    expect(findMessageIds("messageId('y')").ids).toEqual(["y"]);
  });

  it("is not fooled by a // inside a string", () => {
    const scan = findMessageIds('u.startsWith("http://") ? t("a") : t("b")');
    expect(scan.ids).toEqual(["a", "b"]);
  });

  it("reports an import under another name", () => {
    const scan = findMessageIds('import { t as tr } from "./i18n";');
    expect(scan.problems).toHaveLength(1);
  });
});
