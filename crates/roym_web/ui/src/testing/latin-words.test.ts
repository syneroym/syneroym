import { describe, expect, it } from "vitest";
import { ALLOWED_LATIN, leftoverLatin } from "./latin-words";

describe("leftoverLatin", () => {
  it("finds an English word", () => {
    expect(leftoverLatin("यह Save बटन")).toEqual(["Save"]);
  });

  it("does not count a Hindi word or a one-letter word", () => {
    expect(leftoverLatin("यह बटन a")).toEqual([]);
  });

  it("removes a word that holds the data marker", () => {
    expect(leftoverLatin("नाम: Zq-title did:key:zZqAlice")).toEqual([]);
  });

  it("removes an allowed word and a whole allowed command", () => {
    expect(leftoverLatin("Roym और SynOrg में DID")).toEqual([]);
    const command = ALLOWED_LATIN.find((item) => item.text.startsWith("roymctl session"))!;
    expect(leftoverLatin(`चलाएँ \`${command.text}\``)).toEqual([]);
  });

  it("does not let an allowed word hide a longer English word", () => {
    expect(leftoverLatin("Roymcool")).toEqual(["Roymcool"]);
    expect(leftoverLatin("name")).toEqual(["name"]);
    expect(leftoverLatin("ample pmx")).toEqual(["ample", "pmx"]);
    expect(leftoverLatin("1:30 pm")).toEqual([]);
  });

  it("gives each allowed item a reason", () => {
    for (const item of ALLOWED_LATIN) expect(item.why.length).toBeGreaterThan(5);
  });
});
