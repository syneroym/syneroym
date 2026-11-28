import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { modeControl } from "./forward_mode";

describe("the composer mode control", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("starts at the default, changes for one send, and returns to the default", () => {
    const control = modeControl("not-forwardable", false);
    const select = control.root.querySelector<HTMLSelectElement>(".forward-mode-select")!;
    expect(select.value).toBe("not-forwardable");
    expect(control.value()).toBe("not-forwardable");

    select.value = "signed";
    select.dispatchEvent(new Event("change"));
    expect(control.value()).toBe("signed");

    control.reset();
    expect(select.value).toBe("not-forwardable");
    expect(control.value()).toBe("not-forwardable");
  });

  it("takes a new default after the first-message question", () => {
    const control = modeControl("not-forwardable", false);
    const select = control.root.querySelector<HTMLSelectElement>(".forward-mode-select")!;
    control.setDefault("signed");
    expect(select.value).toBe("signed");
    expect(control.value()).toBe("signed");
  });

  it("shows the group notice only for signed in a group", () => {
    const direct = modeControl("signed", false);
    expect(direct.root.querySelector<HTMLElement>(".forward-mode-group-notice")?.style.display).toBe(
      "none",
    );

    const group = modeControl("signed", true);
    expect(group.root.querySelector<HTMLElement>(".forward-mode-group-notice")?.style.display).toBe(
      "",
    );

    const select = group.root.querySelector<HTMLSelectElement>(".forward-mode-select")!;
    select.value = "not-forwardable";
    select.dispatchEvent(new Event("change"));
    expect(group.root.querySelector<HTMLElement>(".forward-mode-group-notice")?.style.display).toBe(
      "none",
    );
  });
});
