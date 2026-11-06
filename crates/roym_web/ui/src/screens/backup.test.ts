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
    { imported: { listings: 4, slots: 1 } },
    { imported: true },
    { imported: { requests: 2 } },
  ];

  async function importLine(index: number): Promise<string> {
    stubRpc({
      "profile.import": () => REPLIES[0],
      "conversation.import": () => REPLIES[1],
      "catalog.import": () => REPLIES[2],
      "transaction.import": () => REPLIES[3],
      "directory.import": () => REPLIES[4],
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
    expect(await importLine(2)).toBe("Imported: 5 record(s).");
    expect(await importLine(3)).toBe("Imported: bundle record(s).");
    expect(await importLine(4)).toBe("Imported: 2 record(s).");
  });
});
