import { describe, expect, it } from "vitest";
import { findVisibleText } from "./scan";

const globbed = import.meta.glob<string>("../**/*.ts", {
  query: "?raw",
  import: "default",
  eager: true,
});
/// The glob names a file of this folder `./x.ts`; make every path `../dir/x.ts`.
const sources = Object.fromEntries(
  Object.entries(globbed).map(([path, source]) => [
    path.startsWith("./") ? `../i18n/${path.slice(2)}` : path,
    source,
  ]),
);

/// Files that set visible text from literals. They are not read by the
/// check yet.
const NOT_MOVED_YET: string[] = [
  "../directory/membership.ts",
  "../listings/editor.ts",
  "../screens/backup.ts",
  "../screens/contacts.ts",
  "../screens/directory.ts",
  "../screens/groups.ts",
  "../screens/listings.ts",
  "../screens/memberships.ts",
  "../screens/messages.ts",
  "../screens/profile.ts",
  "../screens/safety.ts",
  "../screens/setup.ts",
  "../screens/synorg.ts",
  "../screens/synorg_members.ts",
];

/// Files the check does not read: tests, the shared test helpers, and the
/// two files of the string catalog mechanism, whose own texts go only to
/// the console or to tests.
function isChecked(path: string): boolean {
  return (
    !path.endsWith(".test.ts") &&
    !path.startsWith("../testing/") &&
    path !== "../i18n/i18n.ts" &&
    path !== "../i18n/scan.ts"
  );
}

const reported = (code: string): number => findVisibleText(code, "x.ts").length;

describe("the Hub source", () => {
  const checked = Object.entries(sources).filter(([path]) => isChecked(path));

  it("sets no visible text outside the string catalog", () => {
    const problems = checked
      .filter(([path]) => !NOT_MOVED_YET.includes(path))
      .flatMap(([path, source]) => findVisibleText(source, path.replace("../", "src/")));
    expect(problems).toEqual([]);
  });

  it("lists in NOT_MOVED_YET only files that still have visible text", () => {
    for (const path of NOT_MOVED_YET) {
      const source = sources[path];
      expect(source, `${path} exists`).toBeDefined();
      expect(findVisibleText(source, path), `${path} is already clean`).not.toEqual([]);
    }
  });
});

describe("findVisibleText, rule 1: places where text is shown", () => {
  it.each([
    ['el.textContent = "Save"'],
    ['el.placeholder = "x y"'],
    ['el.setAttribute("aria-label", "Close")'],
    ['el.append("Done")'],
    ['text("p", "Hi")'],
    ['field("Name", c)'],
    ['labelled(p, "Payee:", a)'],
    ['labelled(p, t("x"), "after")'],
    ['input("title")'],
    ['checkbox("Taxed", true)'],
    ['new Option("Draft", "draft")'],
    ['throw new HubError("bad")'],
    ['const o = { label: "Profile" }'],
    ['const o = { "hint": "Profile" }'],
    ["alert(`Saved`)"],
  ])("reports %s", (code) => {
    expect(reported(code)).toBe(1);
  });

  it("follows the parts of an expression", () => {
    expect(reported('el.textContent = c ? "Yes" : t("no")')).toBe(1);
    expect(reported('el.textContent = name ?? "Unnamed"')).toBe(1);
    expect(reported("el.textContent = `Saved ${id}`")).toBe(1);
    expect(reported('el.textContent = "Total: " + x')).toBe(1);
    expect(reported('el.textContent = (c ? "A" : "B") as string')).toBe(2);
  });

  it("does not report text with no letter, or text from t()", () => {
    expect(reported('el.textContent = ""')).toBe(0);
    expect(reported('el.textContent = " · "')).toBe(0);
    expect(reported('el.textContent = t("x")')).toBe(0);
    expect(reported("el.textContent = name")).toBe(0);
    expect(reported("el.textContent = `${a} ${b}`")).toBe(0);
  });

  it("does not report a literal that is not shown", () => {
    for (const code of [
      'el.value = "draft"',
      'el.className = "card card-quote"',
      'a.rel = "noopener noreferrer"',
      'console.error("feed listener threw")',
      'el.setAttribute("data-verified", "false")',
      'el.querySelector(".a .b")',
      'el.classList.add("a b")',
      'call("profile.get")',
    ]) {
      expect(reported(code), code).toBe(0);
    }
  });
});

describe("findVisibleText, class lists", () => {
  it("does not report the class list of text()", () => {
    expect(reported('text("p", t("x"), "button bundle-export")')).toBe(0);
    expect(reported('text("span", v, `state-${s} on`)')).toBe(0);
  });

  it("still reports the text of text()", () => {
    expect(reported('text("p", "two words")')).toBe(1);
  });

  it("does not hide a label by its shape", () => {
    expect(reported('const o = { label: "pack size" }')).toBe(1);
    expect(reported('const NAME = "pack size"')).toBe(1);
  });
});

describe("findVisibleText, rule 2: sentences anywhere", () => {
  it("reports a sentence in a constant or a return", () => {
    expect(reported('const NOTE = "This is a sentence."')).toBe(1);
    expect(reported('function f() { return "never checked"; }')).toBe(1);
  });

  it("does not report one word, or a comment", () => {
    expect(reported('const h = "Bearer " + token')).toBe(0);
    expect(reported("// a sentence in a comment\n/* and another one */")).toBe(0);
  });

  it("does not read a type", () => {
    expect(reported('type K = "two words"; const x = y as "a b"')).toBe(0);
  });

  it("reports a literal once when both rules see it", () => {
    expect(reported('el.textContent = "Two words"')).toBe(1);
  });
});

describe("findVisibleText, the problem text", () => {
  it("has the file, the line and the text", () => {
    const [problem] = findVisibleText('const a = 1;\nel.textContent = "Hello";', "src/a.ts");
    expect(problem).toBe('src/a.ts:2: "Hello" is visible text; use t()');
  });
});
