import { afterEach, describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import { importedCount, renderBackup } from "./backup";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("importedCount", () => {
  it("reads a number, adds up a map of counts and ignores anything else", () => {
    expect(importedCount({ imported: 4 })).toBe(4);
    expect(importedCount({ imported: { messages: 3, admissions: 2 } })).toBe(5);
    expect(importedCount({ imported: { things: 0 } })).toBe(0);
    expect(importedCount({ imported: true })).toBeUndefined();
    expect(importedCount({ imported: {} })).toBeUndefined();
    expect(importedCount({ sections: 2 } as never)).toBeUndefined();
    expect(importedCount(null)).toBeUndefined();
    expect(importedCount(undefined)).toBeUndefined();
  });
});

describe("the import line of each bundle", () => {
  /// What each service answers, in the order of the bundle rows.
  const REPLIES = [
    { sections: 3, profiles_verified: 1 },
    { imported: { messages: 3, admissions: 2 } },
    { imported: { listings: 4 } },
    { imported: { availability: 1, reservations: 3 } },
    { imported: true },
    { imported: { requests: 2 } },
  ];

  async function importLine(index: number): Promise<string> {
    stubRpc({
      "profile.import": () => REPLIES[0],
      "conversation.import": () => REPLIES[1],
      "catalog.import": () => REPLIES[2],
      "scheduling.import": () => REPLIES[3],
      "transaction.import": () => REPLIES[4],
      "directory.import": () => REPLIES[5],
    });
    const host = document.createElement("div");
    await renderBackup(host);
    const row = host.querySelectorAll(".bundle-row")[index];
    const file = row.querySelector<HTMLInputElement>("input[type=file]")!;
    Object.defineProperty(file, "files", { value: [{ text: async () => "{}" }] });
    file.dispatchEvent(new Event("change"));
    await vi.waitFor(() => expect(row.querySelector(".bundle-status")?.textContent).toMatch(/^Imported/));
    return row.querySelector(".bundle-status")!.textContent!;
  }

  it("shows a count only when the service reports one, and never a placeholder", async () => {
    expect(await importLine(0)).toBe("Imported: bundle record(s).");
    expect(await importLine(1)).toBe("Imported: 5 record(s).");
    expect(await importLine(2)).toBe("Imported: 4 record(s).");
    expect(await importLine(3)).toBe("Imported: 4 record(s).");
    expect(await importLine(4)).toBe("Imported: bundle record(s).");
    expect(await importLine(5)).toBe("Imported: 2 record(s).");
  });
});

describe("the bundle rows", () => {
  it("lists scheduling above transaction, so the seats are restored first", async () => {
    stubRpc({});
    const host = document.createElement("div");
    await renderBackup(host);
    const exports = [...host.querySelectorAll<HTMLElement>(".bundle-row")].map((r) => r.dataset.export);
    expect(exports).toEqual([
      "profile.export",
      "conversation.export",
      "catalog.export",
      "scheduling.export",
      "transaction.export",
      "directory.export",
    ]);
  });
});

describe("the import line when booked slots could not be restored", () => {
  async function transactionLine(reply: object): Promise<string> {
    stubRpc({ "transaction.import": () => reply });
    const host = document.createElement("div");
    await renderBackup(host);
    const row = host.querySelector<HTMLElement>('.bundle-row[data-export="transaction.export"]')!;
    const file = row.querySelector<HTMLInputElement>("input[type=file]")!;
    Object.defineProperty(file, "files", { value: [{ text: async () => "{}" }] });
    file.dispatchEvent(new Event("change"));
    await vi.waitFor(() => expect(row.querySelector(".bundle-status")?.textContent).toMatch(/^Imported/));
    return row.querySelector(".bundle-status")!.textContent!;
  }

  it("says which file to import again, only when a slot failed", async () => {
    const failed = { imported: true, reservations_restored: 0, reservations_failed: [{ agreement: "a", commitment_id: "c1", reason: "slot-unavailable" }] };
    expect(await transactionLine(failed)).toBe(
      "Imported: bundle record(s). Some booked slots could not be restored. Import the Scheduling file, then import the Requests, offers, and agreements file again.",
    );
    expect(await transactionLine({ imported: true, reservations_restored: 2, reservations_failed: [] })).toBe(
      "Imported: bundle record(s).",
    );
  });
});
