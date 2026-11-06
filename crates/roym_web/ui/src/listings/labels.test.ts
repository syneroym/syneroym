import { describe, expect, it } from "vitest";
import { activateWith, activate } from "../i18n/i18n";
import HINDI from "../i18n/hi.ftl?raw";
import {
  OPEN_TO_KINDS,
  SERVICE_LOCATION_KINDS,
  openToLabel,
  serviceLocationCardLabel,
  serviceLocationEditorLabel,
} from "./labels";

describe("listing labels", () => {
  it("names every place and audience the editor offers", () => {
    for (const kind of SERVICE_LOCATION_KINDS) {
      expect(serviceLocationEditorLabel(kind)).not.toBe(kind);
      expect(serviceLocationCardLabel(kind)).not.toBe(kind);
    }
    for (const kind of OPEN_TO_KINDS) {
      expect(openToLabel(kind)).not.toBe(kind);
    }
  });

  it("reads the places from the person who gives the work", () => {
    expect(serviceLocationEditorLabel("at-giver")).toBe("At my place");
    expect(serviceLocationEditorLabel("at-receiver")).toBe("At their place");
    expect(openToLabel("past-counterparties")).toBe("People I have worked with before");
  });

  it("shows a place from another node as it came, never as undefined", () => {
    expect(serviceLocationCardLabel("at-giver")).toBe("At the quote author's place");
    expect(serviceLocationCardLabel("somewhere-else")).toBe("somewhere-else");
  });

  it("shows an inherited property name as it came", () => {
    for (const value of ["constructor", "toString", "__proto__"]) {
      expect(serviceLocationCardLabel(value)).toBe(value);
    }
  });

  it("follows the Hub language", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    try {
      expect(serviceLocationEditorLabel("at-giver")).toBe("मेरी जगह पर");
    } finally {
      await activate("en");
    }
  });
});
