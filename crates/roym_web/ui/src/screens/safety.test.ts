import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { renderSafety } from "./safety";

afterEach(async () => {
  vi.unstubAllGlobals();
  await activate("en");
});

function report(category: string, kind: string, status: string) {
  return { report_id: "r1", subject_kind: kind, subject_id: "Zq-id", category, status };
}

async function reportRows(rows: unknown[]): Promise<string[]> {
  stubRpc({
    "profile.policy": () => ({ statement: "Zq policy" }),
    "block.list": () => [],
    "report.list": () => rows,
    "contacts.limits": () => ({ window_secs: 60, max_per_window: 3 }),
  });
  const host = document.createElement("div");
  await renderSafety(host);
  await vi.waitFor(() => expect(host.querySelector(".report-row")).not.toBeNull());
  return [...host.querySelectorAll(".report-row span")].map((el) => el.textContent ?? "");
}

describe("the report list", () => {
  it("shows the codes as they are in English", async () => {
    expect(await reportRows([report("fraud", "person", "open")])).toEqual([
      "fraud - person Zq-id [open]",
    ]);
  });

  it("shows each code as a Hindi word", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    const [row] = await reportRows([report("fraud", "person", "open")]);
    expect(row).not.toContain("fraud");
    expect(row).not.toContain("person");
    expect(row).not.toContain("[open]");
    expect(row).toMatch(/[ऀ-ॿ]/);
  });

  it("shows a code it does not know, or an inherited name, as it came", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    const rows = await reportRows([
      report("constructor", "toString", "__proto__"),
      report("new-kind", "listing", "open"),
    ]);
    expect(rows[0]).toContain("constructor - toString Zq-id [__proto__]");
    expect(rows[1]).toContain("new-kind");
  });

  it("offers the codes as the option values", async () => {
    stubRpc({
      "profile.policy": () => ({}),
      "block.list": () => [],
      "report.list": () => [],
      "contacts.limits": () => ({ window_secs: 60, max_per_window: 3 }),
    });
    const host = document.createElement("div");
    await renderSafety(host);
    const values = [...host.querySelectorAll("select option")].map((o) => (o as HTMLOptionElement).value);
    expect(values).toContain("harassment");
    expect(values).toContain("person");
  });
});
