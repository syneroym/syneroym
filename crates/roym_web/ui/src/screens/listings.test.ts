import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { renderListings, verifyParams } from "./listings.js";

describe("listings verifyParams", () => {
  it("never sends an object and sends trimmed text", () => {
    const raw = '  {"envelope": "test"}  \n';
    const params = verifyParams(raw);
    expect(typeof params.envelope).toBe("string");
    expect(params.envelope).toBe('{"envelope": "test"}');
  });
});

describe("the listing list", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  function row(status: string) {
    return {
      listing_id: "l1",
      slug: "Zq-slug",
      title: "Zq-title",
      status,
      record_id: "r1",
      updated_at_secs: 1,
      version_count: 2,
    };
  }

  async function statusWords(...statuses: string[]): Promise<string[]> {
    stubRpc({
      "listing.list": () => ({ listings: statuses.map(row) }),
      "availability.list": () => ({ slots: [] }),
    });
    const host = document.createElement("div");
    await renderListings(host);
    return [...host.querySelectorAll(".listing-status")].map((el) => el.textContent ?? "");
  }

  it("shows a status as its code in English", async () => {
    expect(await statusWords("active", "draft")).toEqual(["active", "draft"]);
  });

  it("shows a status as a Hindi word, and a status it does not know as it came", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    const words = await statusWords("active", "constructor", "toString", "__proto__");
    expect(words[0]).toMatch(/[ऀ-ॿ]/);
    expect(words.slice(1)).toEqual(["constructor", "toString", "__proto__"]);
  });

  it("keeps the code as the option value and shows a word in the option", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    stubRpc({ "listing.list": () => ({ listings: [] }) });
    const host = document.createElement("div");
    await renderListings(host);
    const options = [...host.querySelectorAll<HTMLOptionElement>(".listing-editor select option")];
    expect(options.map((o) => o.value)).toContain("per-hour");
    const draft = options.find((o) => o.value === "draft");
    expect(draft?.textContent).toMatch(/[ऀ-ॿ]/);
  });

  it("gives every option of the editor tables a Hindi word that differs from English", async () => {
    stubRpc({ "listing.list": () => ({ listings: [] }) });
    async function labels(): Promise<string[]> {
      const host = document.createElement("div");
      await renderListings(host);
      const wanted = ["enquiry", "per-hour", "refurbished", "on-agreement", "active"];
      return [...host.querySelectorAll<HTMLOptionElement>(".listing-editor select option")]
        .filter((o) => wanted.includes(o.value))
        .map((o) => `${o.value}=${o.textContent}`);
    }
    const english = await labels();
    expect(english).toHaveLength(5);
    await activateWith("hi", { hi: async () => HINDI });
    const hindi = await labels();
    expect(hindi).toHaveLength(5);
    hindi.forEach((entry, i) => {
      expect(entry.split("=")[0]).toBe(english[i].split("=")[0]);
      expect(entry.split("=")[1]).not.toBe(english[i].split("=")[1]);
    });
  });
});
