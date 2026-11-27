import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { askOnce } from "./first_message";
import { resetForwardSettingsCache, type ForwardMode } from "./forward_mode";

interface Saved {
  default_mode?: ForwardMode;
  asked?: boolean;
}

function settingsNode(initial: { default_mode: ForwardMode; asked: boolean }) {
  const settings = { ...initial };
  const saved: Saved[] = [];
  stubRpc({
    "conversation.settings": (params) => {
      if (Object.keys(params as Record<string, unknown>).length > 0) {
        saved.push(params as Saved);
      }
      if (params["default_mode"] !== undefined) {
        settings.default_mode = params["default_mode"] as ForwardMode;
      }
      if (params["asked"] !== undefined) settings.asked = params["asked"] as boolean;
      return { ...settings };
    },
  });
  return saved;
}

describe("the first-message question", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    resetForwardSettingsCache();
    await activate("en");
  });

  it("asks once and saves the signed choice", async () => {
    const saved = settingsNode({ default_mode: "not-forwardable", asked: false });
    const host = document.createElement("div");
    const answer = askOnce(host);
    await vi.waitFor(() => expect(host.querySelector(".forward-mode-dialog")).not.toBeNull());

    host.querySelector<HTMLButtonElement>(".forward-first-signed")!.click();
    expect(await answer).toBe("signed");
    expect(saved).toEqual([{ default_mode: "signed", asked: true }]);
    expect(host.querySelector(".forward-mode-dialog")).toBeNull();
  });

  it("saves the not-forwardable choice", async () => {
    const saved = settingsNode({ default_mode: "signed", asked: false });
    const host = document.createElement("div");
    const answer = askOnce(host);
    await vi.waitFor(() => expect(host.querySelector(".forward-mode-dialog")).not.toBeNull());

    host.querySelector<HTMLButtonElement>(".forward-first-not-forwardable")!.click();
    expect(await answer).toBe("not-forwardable");
    expect(saved).toEqual([{ default_mode: "not-forwardable", asked: true }]);
  });

  it("skip keeps the default and only marks the question asked", async () => {
    const saved = settingsNode({ default_mode: "not-forwardable", asked: false });
    const host = document.createElement("div");
    const answer = askOnce(host);
    await vi.waitFor(() => expect(host.querySelector(".forward-mode-dialog")).not.toBeNull());

    host.querySelector<HTMLButtonElement>(".forward-first-skip")!.click();
    expect(await answer).toBe("not-forwardable");
    expect(saved).toEqual([{ asked: true }]);
  });

  it("closing the question counts as skip", async () => {
    const saved = settingsNode({ default_mode: "not-forwardable", asked: false });
    const host = document.createElement("div");
    const answer = askOnce(host);
    await vi.waitFor(() => expect(host.querySelector(".forward-mode-dialog")).not.toBeNull());

    host.querySelector<HTMLButtonElement>(".forward-first-close")!.click();
    expect(await answer).toBe("not-forwardable");
    expect(saved).toEqual([{ asked: true }]);
  });

  it("draws no choice as selected", async () => {
    settingsNode({ default_mode: "signed", asked: false });
    const host = document.createElement("div");
    const answer = askOnce(host);
    await vi.waitFor(() => expect(host.querySelector(".forward-mode-dialog")).not.toBeNull());

    expect(host.querySelector(".forward-mode-dialog .selected")).toBeNull();
    host.querySelector<HTMLButtonElement>(".forward-first-skip")!.click();
    await answer;
  });

  it("is not shown again once asked", async () => {
    settingsNode({ default_mode: "signed", asked: true });
    const host = document.createElement("div");
    expect(await askOnce(host)).toBeNull();
    expect(host.querySelector(".forward-mode-dialog")).toBeNull();
  });
});
