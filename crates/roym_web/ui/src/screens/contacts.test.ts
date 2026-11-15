import { afterEach, describe, expect, it, vi } from "vitest";

import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith, t } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { renderContacts, upsertParams } from "./contacts.js";

afterEach(async () => {
  vi.unstubAllGlobals();
  await activate("en");
});

describe("contacts upsertParams", () => {
  it("sends text as the same string and never sends a parsed object", () => {
    const rawEnvelope = '{"record":"profile","display_name":"Alice"}';
    const params = upsertParams("did:key:zAlice", "addr-123", rawEnvelope);
    expect(typeof params.profile_envelope).toBe("string");
    expect(params.profile_envelope).toBe(rawEnvelope);
  });

  it("does not send profile_envelope when empty or whitespace-only", () => {
    const emptyParams = upsertParams("did:key:zAlice", "addr-123", "");
    expect("profile_envelope" in emptyParams).toBe(false);

    const wsParams = upsertParams("did:key:zAlice", "addr-123", "   \n\t  ");
    expect("profile_envelope" in wsParams).toBe(false);
  });

  it("trims address and omits it when empty or whitespace-only", () => {
    const withAddr = upsertParams("  did:key:zAlice  ", "  addr-456  ", "");
    expect(withAddr.person_did).toBe("did:key:zAlice");
    expect(withAddr.conversation_address).toBe("addr-456");

    const noAddr = upsertParams("did:key:zAlice", "", "");
    expect("conversation_address" in noAddr).toBe(false);

    const wsAddr = upsertParams("did:key:zAlice", "   ", "");
    expect("conversation_address" in wsAddr).toBe(false);
  });
});

describe("renderContacts search", () => {
  it("has button with type=button and performs search lifecycle", async () => {
    const calls: unknown[] = [];
    stubRpc({
      "contacts.list": (params: unknown) => {
        calls.push(params);
        if (
          params &&
          typeof params === "object" &&
          "text" in params &&
          (params as { text: string }).text === "Ram"
        ) {
          return [{ person_did: "did:key:zRam", display_name: "Ram" }];
        }
        if (
          params &&
          typeof params === "object" &&
          "text" in params &&
          (params as { text: string }).text === "Nobody"
        ) {
          return [];
        }
        return [
          { person_did: "did:key:zRam", display_name: "Ram" },
          { person_did: "did:key:zSita", display_name: "Sita" },
        ];
      },
    });

    const host = document.createElement("div");
    await renderContacts(host);

    const searchBtn = host.querySelector<HTMLButtonElement>(".contacts-search-button")!;
    expect(searchBtn).not.toBeNull();
    expect(searchBtn.type).toBe("button");

    const searchInput = host.querySelector<HTMLInputElement>(".contacts-search-input")!;
    expect(searchInput).not.toBeNull();

    // Initial load calls contacts.list with {}
    expect(calls[0]).toEqual({});

    // 1. Search reloads with { text }
    searchInput.value = "Ram";
    searchBtn.click();
    await vi.waitFor(() => {
      expect(calls).toContainEqual({ text: "Ram" });
      expect(host.textContent).toContain("Ram");
    });

    // 2. Enter triggers search
    searchInput.value = "Nobody";
    searchInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await vi.waitFor(() => {
      expect(calls).toContainEqual({ text: "Nobody" });
      // 3. An empty answer shows contacts-search-empty
      expect(host.textContent).toContain(t("contacts-search-empty"));
    });

    // 4. Clearing search reloads with {}
    searchInput.value = "";
    searchBtn.click();
    await vi.waitFor(() => {
      expect(calls[calls.length - 1]).toEqual({});
      expect(host.textContent).toContain("Sita");
    });

    // 5. Add form has distinct class names
    expect(host.querySelector(".contacts-did-input")).not.toBeNull();
    expect(host.querySelector(".contacts-address-input")).not.toBeNull();
    expect(host.querySelector(".contacts-envelope-input")).not.toBeNull();
    expect(host.querySelector(".contacts-upsert-button")).not.toBeNull();
  });
});

describe("renderContacts order", () => {
  async function drawnNames(): Promise<string[]> {
    stubRpc({
      "contacts.list": () => [
        { person_did: "did:key:zSita", display_name: "Sita" },
        { person_did: "did:key:zRamDeva", display_name: "राम" },
        { person_did: "did:key:zRamLatin", display_name: "Ram" },
      ],
    });
    const host = document.createElement("div");
    await renderContacts(host);
    const list = host.querySelector<HTMLElement>(".contacts-screen > div")!;
    return Array.from(list.children).map((row) => row.textContent!.trim());
  }

  it("lists Latin names first in an English Hub", async () => {
    await activate("en");
    expect(await drawnNames()).toEqual(["Ram", "Sita", "राम"]);
  });

  it("lists Devanagari names first in a Hindi Hub", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    expect(await drawnNames()).toEqual(["राम", "Ram", "Sita"]);
  });
});
