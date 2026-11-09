import base from "../../../../roym_core/taxonomy/base.json";
import { codeWord, formatLocale, lookupText, tEnglish } from "../i18n/i18n";

export interface CategoryGroup {
  code: string;
  children: string[];
}

interface RawCategory {
  code: string;
  parent?: string;
}

interface RawAttributeChoice {
  type: "choice";
  choices: string[];
}

interface RawAttributeNumber {
  type: "number";
  unit?: string;
}

interface RawAttributeOther {
  type: "yes-no" | "text";
}

type RawAttributeValue = RawAttributeChoice | RawAttributeNumber | RawAttributeOther;

interface RawAttribute {
  code: string;
  categories: string[];
  value: RawAttributeValue;
}

interface RawTaxonomy {
  categories: RawCategory[];
  units: string[];
  payment_methods: string[];
  attributes: RawAttribute[];
}

const taxonomyData = base as RawTaxonomy;

const groups: CategoryGroup[] = [];
const groupMap = new Map<string, CategoryGroup>();
const parentOfChild = new Map<string, string>();
const allChildren: { code: string; parent: string }[] = [];

for (const c of taxonomyData.categories) {
  if (c.parent == null) {
    const group: CategoryGroup = { code: c.code, children: [] };
    groups.push(group);
    groupMap.set(c.code, group);
  } else {
    parentOfChild.set(c.code, c.parent);
    allChildren.push({ code: c.code, parent: c.parent });
    const group = groupMap.get(c.parent);
    if (group) {
      group.children.push(c.code);
    }
  }
}

export const CATEGORY_GROUPS: readonly CategoryGroup[] = groups;
export const UNITS: readonly string[] = taxonomyData.units;
export const PAYMENT_METHODS: readonly string[] = taxonomyData.payment_methods;

const CATEGORY_IDS = new Map<string, string>(
  taxonomyData.categories.map((c) => [c.code, "taxonomy-category-" + c.code]),
);

const UNIT_IDS = new Map<string, string>(
  taxonomyData.units.map((u) => [u, "taxonomy-unit-" + u]),
);

const METHOD_IDS = new Map<string, string>(
  taxonomyData.payment_methods.map((m) => [m, "taxonomy-method-" + m]),
);

const ATTRIBUTE_IDS = new Map<string, string>(
  taxonomyData.attributes.map((a) => [a.code, "taxonomy-attribute-" + a.code]),
);

const CHOICE_IDS = new Map<string, string>();
for (const a of taxonomyData.attributes) {
  if (a.value.type === "choice") {
    for (const choice of a.value.choices) {
      CHOICE_IDS.set(`${a.code}_${choice}`, `taxonomy-choice-${a.code}_${choice}`);
    }
  }
}

export function isTaxonomyCategory(code: string): boolean {
  return CATEGORY_IDS.has(code);
}

export function categoryGroupOf(code: string): string | undefined {
  return parentOfChild.get(code);
}

export function categoryLabel(code: string): string {
  return codeWord(CATEGORY_IDS, code);
}

export function unitLabel(code: string): string {
  return codeWord(UNIT_IDS, code);
}

export function methodLabel(code: string): string {
  return codeWord(METHOD_IDS, code);
}

export function attributeLabel(code: string): string {
  return codeWord(ATTRIBUTE_IDS, code);
}

export function choiceLabel(attribute: string, choice: string): string {
  return lookupText(CHOICE_IDS, `${attribute}_${choice}`) ?? choice;
}

export function taxonomyMessageIds(): string[] {
  return [
    ...CATEGORY_IDS.values(),
    ...UNIT_IDS.values(),
    ...METHOD_IDS.values(),
    ...ATTRIBUTE_IDS.values(),
    ...CHOICE_IDS.values(),
  ];
}

function englishCategoryLabel(code: string): string {
  const id = CATEGORY_IDS.get(code);
  return id ? tEnglish(id) : code;
}

export function matchCategories(text: string): string[] {
  const locale = formatLocale();
  const query = text.trim().toLocaleLowerCase(locale);
  if (!query) {
    return allChildren.map((c) => c.code);
  }
  return allChildren
    .filter((c) => {
      if (c.code.toLocaleLowerCase(locale).includes(query)) return true;
      if (categoryLabel(c.code).toLocaleLowerCase(locale).includes(query)) return true;
      if (englishCategoryLabel(c.code).toLocaleLowerCase(locale).includes(query)) return true;
      if (c.parent.toLocaleLowerCase(locale).includes(query)) return true;
      if (categoryLabel(c.parent).toLocaleLowerCase(locale).includes(query)) return true;
      if (englishCategoryLabel(c.parent).toLocaleLowerCase(locale).includes(query)) return true;
      return false;
    })
    .map((c) => c.code);
}
