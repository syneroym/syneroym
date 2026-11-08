import { afterEach, describe, expect, it } from "vitest";
import base from "../../../../roym_core/taxonomy/base.json";
import { activate, activateWith } from "../i18n/i18n";
import {
  attributeLabel,
  CATEGORY_GROUPS,
  categoryGroupOf,
  categoryLabel,
  choiceLabel,
  isTaxonomyCategory,
  matchCategories,
  methodLabel,
  unitLabel,
  UNITS,
  PAYMENT_METHODS,
} from "./taxonomy";

describe("taxonomy labels and helpers", () => {
  afterEach(async () => {
    await activate("en");
  });

  it("shows English labels, and in Hindi different text with a Devanagari letter", async () => {
    await activate("en");
    expect(categoryLabel("plumbing")).toBe("Plumbing");
    expect(unitLabel("kg")).toBe("kg");
    expect(methodLabel("upi")).toBe("UPI");
    expect(attributeLabel("diet")).toBe("Diet");
    expect(choiceLabel("diet", "veg")).toBe("Vegetarian");

    await activate("hi");
    const hiCat = categoryLabel("plumbing");
    expect(hiCat).not.toBe("Plumbing");
    expect(/[ऀ-ॿ]/.test(hiCat)).toBe(true);

    const hiUnit = unitLabel("kg");
    expect(hiUnit).not.toBe("kg");
    expect(/[ऀ-ॿ]/.test(hiUnit)).toBe(true);

    const hiMethod = methodLabel("upi");
    expect(hiMethod).not.toBe("UPI");
    expect(/[ऀ-ॿ]/.test(hiMethod)).toBe(true);

    const hiAttr = attributeLabel("diet");
    expect(hiAttr).not.toBe("Diet");
    expect(/[ऀ-ॿ]/.test(hiAttr)).toBe(true);

    const hiChoice = choiceLabel("diet", "veg");
    expect(hiChoice).not.toBe("Vegetarian");
    expect(/[ऀ-ॿ]/.test(hiChoice)).toBe(true);
  });

  it("falls back to English when a language lacks taxonomy labels", async () => {
    await activateWith("xx", {
      xx: async () => "dummy-msg = Dummy",
    });
    expect(categoryLabel("plumbing")).toBe("Plumbing");
    expect(unitLabel("kg")).toBe("kg");
    expect(methodLabel("upi")).toBe("UPI");
    expect(attributeLabel("diet")).toBe("Diet");
    expect(choiceLabel("diet", "veg")).toBe("Vegetarian");
  });

  it("shows unknown codes as they came, for each of the five label functions", () => {
    expect(categoryLabel("zz-own")).toBe("zz-own");
    expect(unitLabel("zz-own")).toBe("zz-own");
    expect(methodLabel("zz-own")).toBe("zz-own");
    expect(attributeLabel("zz-own")).toBe("zz-own");
    expect(choiceLabel("diet", "zz-own")).toBe("zz-own");

    for (const inherited of ["constructor", "toString", "__proto__"]) {
      expect(categoryLabel(inherited)).toBe(inherited);
      expect(unitLabel(inherited)).toBe(inherited);
      expect(methodLabel(inherited)).toBe(inherited);
      expect(attributeLabel(inherited)).toBe(inherited);
      expect(choiceLabel("diet", inherited)).toBe(inherited);
      expect(choiceLabel(inherited, inherited)).toBe(inherited);
    }
  });

  it("matches categories by child code, child label, group code, and group label", async () => {
    await activate("en");
    expect(matchCategories("plumb")).toContain("plumbing");
    expect(matchCategories("PLUMB")).toContain("plumbing");

    // A group code is never returned
    for (const group of CATEGORY_GROUPS) {
      expect(matchCategories(group.code)).not.toContain(group.code);
    }

    // Group label in English finds all children of that group
    const healthChildren = matchCategories("Health");
    expect(healthChildren).toContain("doctor");
    expect(healthChildren).toContain("nursing");
    expect(healthChildren).toContain("yoga");
    expect(healthChildren).not.toContain("health");

    // Result keeps file order
    const all = matchCategories("");
    const baseChildren = base.categories
      .filter((c: { code: string; parent?: string }) => c.parent != null)
      .map((c: { code: string }) => c.code);
    expect(all).toEqual(baseChildren);

    // Hindi label matching in Hindi
    await activate("hi");
    expect(matchCategories("प्लंबिंग")).toContain("plumbing");
    const hiHealthChildren = matchCategories("स्वास्थ्य");
    expect(hiHealthChildren).toContain("doctor");
    expect(hiHealthChildren).toContain("nursing");
    expect(hiHealthChildren).toContain("yoga");
  });

  it("builds CATEGORY_GROUPS with every category of base.json", () => {
    const totalCategories = base.categories.length;
    let counted = 0;
    for (const group of CATEGORY_GROUPS) {
      counted += 1 + group.children.length;
      expect(isTaxonomyCategory(group.code)).toBe(true);
      expect(categoryGroupOf(group.code)).toBeUndefined();
      for (const child of group.children) {
        expect(isTaxonomyCategory(child)).toBe(true);
        expect(categoryGroupOf(child)).toBe(group.code);
      }
    }
    expect(counted).toBe(totalCategories);
    expect(UNITS.length).toBe(base.units.length);
    expect(PAYMENT_METHODS.length).toBe(base.payment_methods.length);
  });
});
