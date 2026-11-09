import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { PAYMENT_METHODS, unitLabel, UNITS } from "../taxonomy/taxonomy";
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

  it("unit list starts at piece and has one option per unit with its label", async () => {
    stubRpc({ "listing.list": () => ({ listings: [] }) });
    const host = document.createElement("div");
    await renderListings(host);
    const productBlock = host.querySelector(".block-product");
    const unitSelect = productBlock?.querySelector("select");
    expect(unitSelect).not.toBeNull();
    expect(unitSelect?.value).toBe("piece");
    const options = [...(unitSelect?.querySelectorAll("option") ?? [])];
    expect(options.map((o) => o.value)).toEqual([...UNITS]);
    expect(options.map((o) => o.textContent)).toEqual(UNITS.map((u) => unitLabel(u)));
  });

  it("sends the ticked payment methods in list order and chosen categories on save", async () => {
    let savedParams: Record<string, unknown> | undefined;
    stubRpc({
      "listing.list": () => ({ listings: [] }),
      "listing.set": (params) => {
        savedParams = params;
        return { listing_id: "l1", record_id: "r1", version_count: 1 };
      },
    });
    const host = document.createElement("div");
    await renderListings(host);

    // Verify all payment methods are present as checkboxes
    const methodBoxes = [...host.querySelectorAll<HTMLInputElement>(".payment-methods input[type='checkbox']")];
    expect(methodBoxes.map((b) => b.value)).toEqual([...PAYMENT_METHODS]);

    const title = host.querySelector<HTMLInputElement>(".listing-title-input")!;
    title.value = "Test Title";

    // Category picker: pick "plumbing"
    const filter = host.querySelector<HTMLInputElement>(".listing-categories-input .picker-filter")!;
    filter.value = "Plumbing";
    filter.dispatchEvent(new Event("input"));
    const opt = host.querySelector<HTMLButtonElement>('.picker-option[data-code="plumbing"]')!;
    opt.click();

    // Enable payment block
    const payBlock = host.querySelector(".block-payment")!;
    const payEnabled = payBlock.querySelector<HTMLInputElement>(".block-enabled")!;
    payEnabled.checked = true;
    const amountInput = payBlock.querySelector<HTMLInputElement>(".payment-amount-input")!;
    amountInput.value = "10.00";

    // Tick upi and cash out of order
    const upiBox = payBlock.querySelector<HTMLInputElement>(".payment-method-upi")!;
    const cashBox = payBlock.querySelector<HTMLInputElement>(".payment-method-cash")!;
    upiBox.checked = true;
    cashBox.checked = true;

    const saveBtn = host.querySelector<HTMLButtonElement>(".save-listing")!;
    saveBtn.click();
    await vi.waitFor(() => expect(savedParams).toBeDefined());

    expect(savedParams!.categories).toEqual(["plumbing"]);
    const payment = savedParams!.payment as Record<string, unknown>;
    expect(payment.methods).toEqual(["cash", "upi"]);
  });

  it("status select has class listing-status-select and translation rows add/remove/cap at 4", async () => {
    stubRpc({ "listing.list": () => ({ listings: [] }) });
    const host = document.createElement("div");
    await renderListings(host);

    const statusSel = host.querySelector(".listing-status-select");
    expect(statusSel).not.toBeNull();

    const addBtn = host.querySelector<HTMLButtonElement>(".listing-translation-add-btn")!;
    expect(addBtn).not.toBeNull();
    expect(addBtn.disabled).toBe(false);

    expect(host.querySelectorAll(".listing-translation-row")).toHaveLength(0);

    // Add row 1
    addBtn.click();
    expect(host.querySelectorAll(".listing-translation-row")).toHaveLength(1);
    expect(addBtn.disabled).toBe(false);

    // Add row 2, 3, 4
    addBtn.click();
    addBtn.click();
    addBtn.click();
    expect(host.querySelectorAll(".listing-translation-row")).toHaveLength(4);
    expect(addBtn.disabled).toBe(true);

    // Remove one row
    const rmBtn = host.querySelector<HTMLButtonElement>(".listing-translation-remove-btn")!;
    rmBtn.click();
    expect(host.querySelectorAll(".listing-translation-row")).toHaveLength(3);
    expect(addBtn.disabled).toBe(false);
  });
});
