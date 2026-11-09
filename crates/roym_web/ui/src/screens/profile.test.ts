import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { renderProfile } from "./profile";

describe("profile screen", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("new profile starts text language and languages list with UI language", async () => {
    stubRpc({
      "profile.get": () => ({ envelope: null }),
    });

    const host = document.createElement("div");
    await renderProfile(host);

    const langSel = host.querySelector<HTMLSelectElement>(".profile-text-lang");
    expect(langSel).not.toBeNull();
    expect(langSel?.value).toBe("en");

    const rows = host.querySelectorAll(".profile-language-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector(".profile-language-name")?.textContent).toBe("English");
  });

  it("stored profile with lang and languages displays both in order", async () => {
    const payload = {
      display_name: "Alice",
      conversation_address: "did:key:alice#conv",
      lang: "ta",
      languages: ["ta", "en"],
    };
    stubRpc({
      "profile.get": () => ({
        envelope: JSON.stringify({ payload }),
      }),
    });

    const host = document.createElement("div");
    await renderProfile(host);

    const langSel = host.querySelector<HTMLSelectElement>(".profile-text-lang");
    expect(langSel?.value).toBe("ta");

    const rows = host.querySelectorAll(".profile-language-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".profile-language-name")?.textContent).toBe("Tamil");
    expect(rows[1].querySelector(".profile-language-name")?.textContent).toBe("English");
  });

  it("add, move up, and remove change languages order and add list is disabled at 8", async () => {
    let savedParams: Record<string, unknown> | undefined;
    stubRpc({
      "profile.get": () => ({ envelope: null }),
      "profile.set": (params) => {
        savedParams = params as Record<string, unknown>;
        return { record_id: "rec_1", envelope: "{}" };
      },
    });

    const host = document.createElement("div");
    await renderProfile(host);

    const addSel = host.querySelector<HTMLSelectElement>(".profile-languages-add")!;
    expect(addSel).not.toBeNull();
    // English is already in the list, so options must not contain en
    const optionValues = Array.from(addSel.options).map((o) => o.value);
    expect(optionValues).not.toContain("en");

    // Add Hindi
    addSel.value = "hi";
    addSel.dispatchEvent(new Event("change"));

    let rows = host.querySelectorAll(".profile-language-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".profile-language-name")?.textContent).toBe("English");
    expect(rows[1].querySelector(".profile-language-name")?.textContent).toBe("Hindi");

    // Move Hindi up
    const upBtn = rows[1].querySelector<HTMLButtonElement>(".profile-language-move-up")!;
    expect(upBtn.type).toBe("button");
    upBtn.click();
    // Form should not submit when clicking move up button
    expect(savedParams).toBeUndefined();

    rows = host.querySelectorAll(".profile-language-row");
    expect(rows[0].querySelector(".profile-language-name")?.textContent).toBe("Hindi");
    expect(rows[1].querySelector(".profile-language-name")?.textContent).toBe("English");

    // Add 6 more languages to reach 8 total: bn, gu, kn, ml, mr, or
    const tagsToAdd = ["bn", "gu", "kn", "ml", "mr", "or"];
    for (const tag of tagsToAdd) {
      addSel.value = tag;
      addSel.dispatchEvent(new Event("change"));
    }
    expect(host.querySelectorAll(".profile-language-row")).toHaveLength(8);
    expect(addSel.disabled).toBe(true);

    // Remove one language
    const firstRowRm = host.querySelector<HTMLButtonElement>(".profile-language-remove")!;
    expect(firstRowRm.type).toBe("button");
    firstRowRm.click();
    expect(host.querySelectorAll(".profile-language-row")).toHaveLength(7);
    expect(addSel.disabled).toBe(false);
  });

  it("save sends lang and languages in order, no locale, and preserves stored about and translations", async () => {
    let savedParams: Record<string, unknown> | undefined;
    const initialPayload = {
      display_name: "Old Name",
      about: "Stored about text",
      translations: [{ lang: "hi", about: "हिंदी विवरण" }],
      lang: "en",
      languages: ["en", "ta"],
    };
    stubRpc({
      "profile.get": () => ({
        envelope: JSON.stringify({ payload: initialPayload }),
      }),
      "profile.set": (params) => {
        savedParams = params as Record<string, unknown>;
        return { record_id: "rec_1", envelope: "{}" };
      },
    });

    const host = document.createElement("div");
    await renderProfile(host);

    const nameInput = host.querySelector<HTMLInputElement>("input[type='text']")!;
    nameInput.value = "New Name";

    const form = host.querySelector<HTMLFormElement>("form")!;
    form.dispatchEvent(new Event("submit", { cancelable: true }));

    await vi.waitFor(() => expect(savedParams).toBeDefined());
    expect(savedParams!.display_name).toBe("New Name");
    expect(savedParams!.lang).toBe("en");
    expect(savedParams!.languages).toEqual(["en", "ta"]);
    expect(savedParams!.locale).toBeUndefined();
    expect(savedParams!.about).toBe("Stored about text");
    expect(savedParams!.translations).toEqual([{ lang: "hi", about: "हिंदी विवरण" }]);
  });
});
