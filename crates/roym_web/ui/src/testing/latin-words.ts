/// Latin-letter text that may stay on a Hindi screen. The list is written
/// here and not read from `hi.ftl`: an English word pasted into a Hindi
/// message would then be allowed everywhere. A new word needs an edit of
/// this list, which a reviewer sees.
export const ALLOWED_LATIN: ReadonlyArray<{ text: string; why: string }> = [
  { text: "Roym", why: "the product name" },
  { text: "SynOrg", why: "the name of a group of members" },
  { text: "DID", why: "the name of an identifier" },
  {
    text: "roymctl session delegate --as <name> --registry-url <registry>",
    why: "a command that the sign-in help shows",
  },
  {
    text: "roymctl roym enrol-signing --master <identity> --registry-url <registry>",
    why: "a command that the set-up help shows",
  },
  { text: "ISO-4217", why: "the name of the currency code standard" },
  { text: "USD", why: "an example currency code in the quote form placeholder" },
  { text: "EUR", why: "the example currency code in the amount placeholder and the fixtures" },
  { text: "roymctl roym backup create", why: "a command that the backup help shows" },
  { text: "JSON", why: "the name of a data format that a person pastes" },
  { text: "HTTP", why: "the protocol name in a line about a failed request" },
  { text: "did:key:...", why: "the start of an identifier, shown as an example of the format" },
  { text: "item", why: "the unit that the listing editor starts with; a code sent to the service" },
  { text: "am", why: "`Intl` writes the 12-hour mark of a Hindi time in Latin letters" },
  { text: "pm", why: "`Intl` writes the 12-hour mark of a Hindi time in Latin letters" },
  { text: "UTC", why: "the time zone name that `Intl` writes when the tests run with TZ=UTC" },
  { text: "Fake service error", why: "the detail text of an error from the fake `/rpc`" },
];

/// A value that a person types or another node sends holds this marker in
/// the test fixtures, and is removed before the check. A code is never
/// marked, so a screen that shows a code as it came is found.
export const DATA_MARKER = "Zq";

const LATIN_WORD = /[A-Za-z]{2,}/g;

/// An allowed item matches only as a whole word, so `am` does not hide the
/// start of `ample`.
function wholeWord(allowed: string): RegExp {
  const escaped = allowed.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?<![A-Za-z])${escaped}(?![A-Za-z])`, "g");
}

/// The Latin words of `text` that are neither data (a word with the marker)
/// nor on the allowed list.
export function leftoverLatin(text: string): string[] {
  let rest = text
    .split(/\s+/)
    .filter((word) => !word.includes(DATA_MARKER))
    .join(" ");
  const longestFirst = [...ALLOWED_LATIN].sort((a, b) => b.text.length - a.text.length);
  for (const { text: allowed } of longestFirst) rest = rest.replace(wholeWord(allowed), " ");
  return rest.match(LATIN_WORD) ?? [];
}
