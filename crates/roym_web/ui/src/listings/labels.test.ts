import { describe, expect, it } from "vitest";
import {
  OPEN_TO_KINDS,
  OPEN_TO_LABELS,
  SERVICE_LOCATION_CARD_LABELS,
  SERVICE_LOCATION_EDITOR_LABELS,
  SERVICE_LOCATION_KINDS,
  serviceLocationCardLabel,
} from "./labels";

describe("listing labels", () => {
  it("names every place and audience the editor offers", () => {
    for (const kind of SERVICE_LOCATION_KINDS) {
      expect(SERVICE_LOCATION_EDITOR_LABELS[kind]).toBeTruthy();
      expect(SERVICE_LOCATION_CARD_LABELS[kind]).toBeTruthy();
    }
    for (const kind of OPEN_TO_KINDS) {
      expect(OPEN_TO_LABELS[kind]).toBeTruthy();
    }
  });

  it("reads the places from the person who gives the work", () => {
    expect(SERVICE_LOCATION_EDITOR_LABELS["at-giver"]).toBe("At my place");
    expect(SERVICE_LOCATION_EDITOR_LABELS["at-receiver"]).toBe("At their place");
    expect(OPEN_TO_LABELS["past-counterparties"]).toBe("People I have worked with before");
  });

  it("shows a place from another node as it came, never as undefined", () => {
    expect(serviceLocationCardLabel("at-giver")).toBe("At the quote author's place");
    expect(serviceLocationCardLabel("somewhere-else")).toBe("somewhere-else");
  });
});
