import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { download } from "../download";
import { stubRpc } from "../testing/stub-rpc";
import {
  lastMonthRange,
  parseDateInput,
  renderStatement,
  thisMonthRange,
  thisYearRange,
  toDateInputValue,
  type Statement,
} from "./statement";

vi.mock("../download", () => ({
  download: vi.fn(),
}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

function readBlobText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = reject;
    reader.readAsText(blob);
  });
}

describe("statement pure date helpers", () => {
  it("converts Date to YYYY-MM-DD input string and parses it back", () => {
    const d = new Date(2026, 4, 15, 14, 30);
    const str = toDateInputValue(d);
    expect(str).toBe("2026-05-15");

    const parsed = parseDateInput("2026-05-15");
    expect(parsed).not.toBeNull();
    expect(parsed!.getFullYear()).toBe(2026);
    expect(parsed!.getMonth()).toBe(4);
    expect(parsed!.getDate()).toBe(15);
    expect(parsed!.getHours()).toBe(0);

    expect(parseDateInput("invalid")).toBeNull();
  });

  it("computes thisMonthRange, lastMonthRange, and thisYearRange", () => {
    const fixedNow = new Date(2026, 4, 15, 12, 0, 0); // May 15, 2026
    const thisMonth = thisMonthRange(fixedNow);
    expect(thisMonth.fromStr).toBe("2026-05-01");
    expect(thisMonth.toStr).toBe("2026-05-31");
    expect(thisMonth.from_secs).toBeLessThan(thisMonth.to_secs);

    const lastMonth = lastMonthRange(fixedNow);
    expect(lastMonth.fromStr).toBe("2026-04-01");
    expect(lastMonth.toStr).toBe("2026-04-30");
    expect(lastMonth.from_secs).toBeLessThan(lastMonth.to_secs);

    const thisYear = thisYearRange(fixedNow);
    expect(thisYear.fromStr).toBe("2026-01-01");
    expect(thisYear.toStr).toBe("2026-12-31");
    expect(thisYear.from_secs).toBeLessThan(thisYear.to_secs);
  });

  it("includes the entire 'To' day by adding 1 day to to_secs", async () => {
    let capturedParams: { from_secs: number; to_secs: number } | null = null;
    stubRpc({
      "statement.export": (params: Record<string, unknown>) => {
        capturedParams = params as { from_secs: number; to_secs: number };
        return {
          statement_version: 1,
          owner_did: "did:key:alice",
          from_secs: (params as { from_secs: number }).from_secs,
          to_secs: (params as { to_secs: number }).to_secs,
          generated_at_secs: 1000,
          rows: [],
          open_at_end: [],
          totals: [],
          eeps: { status: "not-available" },
        };
      },
      "contacts.list": () => [],
    });

    const host = document.createElement("div");
    await renderStatement(host);
    const fromInput = host.querySelector<HTMLInputElement>(".statement-from-input")!;
    const toInput = host.querySelector<HTMLInputElement>(".statement-to-input")!;

    fromInput.value = "2026-11-01";
    toInput.value = "2026-11-30";

    host.querySelector<HTMLButtonElement>(".statement-show-button")!.click();
    await vi.waitFor(() => {
      expect(capturedParams).not.toBeNull();
    });

    // Nov 30 23:59:59 is inside [from_secs, to_secs) because to_secs is Dec 01 00:00:00
    const nov30EndOfDay = Math.floor(new Date(2026, 10, 30, 23, 59, 59).getTime() / 1000);
    expect(nov30EndOfDay).toBeLessThan(capturedParams!.to_secs);
    expect(nov30EndOfDay).toBeGreaterThanOrEqual(capturedParams!.from_secs);
  });
});

describe("statement screen", () => {
  let host: HTMLElement;

  beforeEach(() => {
    host = document.createElement("div");
  });

  it("initializes with default period and updates dates on quick button clicks", async () => {
    await renderStatement(host);

    const fromInput = host.querySelector<HTMLInputElement>(".statement-from-input")!;
    const toInput = host.querySelector<HTMLInputElement>(".statement-to-input")!;

    const currentRange = thisMonthRange();
    expect(fromInput.value).toBe(currentRange.fromStr);
    expect(toInput.value).toBe(currentRange.toStr);

    const lastMonthBtn = host.querySelector<HTMLButtonElement>(".statement-period-last-month")!;
    lastMonthBtn.click();
    const lastRange = lastMonthRange();
    expect(fromInput.value).toBe(lastRange.fromStr);
    expect(toInput.value).toBe(lastRange.toStr);

    const thisYearBtn = host.querySelector<HTMLButtonElement>(".statement-period-this-year")!;
    thisYearBtn.click();
    const yearRange = thisYearRange();
    expect(fromInput.value).toBe(yearRange.fromStr);
    expect(toInput.value).toBe(yearRange.toStr);

    const thisMonthBtn = host.querySelector<HTMLButtonElement>(".statement-period-this-month")!;
    thisMonthBtn.click();
    expect(fromInput.value).toBe(currentRange.fromStr);
    expect(toInput.value).toBe(currentRange.toStr);
  });

  it("shows error when start date is after or equal to end date", async () => {
    await renderStatement(host);

    const fromInput = host.querySelector<HTMLInputElement>(".statement-from-input")!;
    const toInput = host.querySelector<HTMLInputElement>(".statement-to-input")!;
    const showBtn = host.querySelector<HTMLButtonElement>(".statement-show-button")!;
    const errorEl = host.querySelector<HTMLElement>(".statement-error")!;

    fromInput.value = "2026-06-01";
    toInput.value = "2026-05-01";
    showBtn.click();

    expect(errorEl.textContent).toBe("Start date must be before end date");

    fromInput.value = "2026-05-01";
    toInput.value = "2026-05-01";
    showBtn.click();

    expect(errorEl.textContent).toBe("Start date must be before end date");
  });

  it("fetches statement, merges counterparty names from contacts, and renders rows, totals, and eeps", async () => {
    const mockStatement: Statement = {
      statement_version: 1,
      owner_did: "did:key:zMe",
      from_secs: 1700000000,
      to_secs: 1700086400,
      generated_at_secs: 1700086400,
      rows: [
        {
          agreement: "agr_1",
          commitment_id: "c1",
          agreed_at_secs: 1700001000,
          settled_at_secs: 1700005000,
          direction: "given",
          counterparty_did: "did:key:zBob",
          resource_kind: "service",
          commitment_kind: "fulfilment",
          description: { lang: "en", text: "Piano lesson" },
          currency: "INR",
          declared_value_minor: 50000,
          settlement_note: { lang: "en", text: "Completed in studio" },
          records: ["rec_1"],
        },
      ],
      open_at_end: [
        {
          agreement: "agr_2",
          commitment_id: "c2",
          agreed_at_secs: 1700050000,
          direction: "received",
          counterparty_did: "did:key:zAlice",
          resource_kind: "goods",
          commitment_kind: "fulfilment",
          description: { lang: "en", text: "Bag of wheat" },
          currency: "INR",
          declared_value_minor: 25000,
        },
      ],
      totals: [
        {
          currency: "INR",
          direction: "given",
          resource_kind: "service",
          sum_minor: 50000,
        },
      ],
      eeps: { status: "not-available" },
    };

    const recorded = stubRpc({
      "statement.export": () => mockStatement,
      "contacts.list": () => [
        { person_did: "did:key:zBob", display_name: "Bob Builder" },
      ],
    });

    await renderStatement(host);
    const fromInput = host.querySelector<HTMLInputElement>(".statement-from-input")!;
    const toInput = host.querySelector<HTMLInputElement>(".statement-to-input")!;
    fromInput.value = "2026-05-01";
    toInput.value = "2026-06-01";

    const showBtn = host.querySelector<HTMLButtonElement>(".statement-show-button")!;
    showBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelectorAll(".statement-row").length).toBe(1);
    });

    expect(recorded.some((c) => c.method === "statement.export")).toBe(true);
    expect(recorded.some((c) => c.method === "contacts.list")).toBe(true);

    const row = host.querySelector(".statement-row")!;
    expect(row.querySelector(".statement-row-counterparty")?.textContent).toBe("Bob Builder");
    expect(row.querySelector(".statement-row-description")?.textContent).toBe("Piano lesson");
    expect(row.querySelector(".statement-row-direction")?.textContent).toBe("Given");
    expect(row.querySelector(".statement-row-value")?.textContent).toContain("500.00");
    expect(row.querySelector(".statement-row-note")?.textContent).toBe("Completed in studio");

    // Open at end
    const openRow = host.querySelector(".statement-open-row")!;
    // Alice is not in contacts list, so fallback to DID
    expect(openRow.querySelector(".statement-row-counterparty")?.textContent).toBe("did:key:zAlice");
    expect(openRow.querySelector(".statement-row-description")?.textContent).toBe("Bag of wheat");

    // Totals
    const totalRow = host.querySelector(".statement-total-row")!;
    expect(totalRow.querySelector(".statement-total-direction")?.textContent).toBe("Given");
    expect(totalRow.querySelector(".statement-total-kind")?.textContent).toBe("Service");
    expect(totalRow.querySelector(".statement-total-sum")?.textContent).toContain("500.00");

    // EEP
    expect(host.querySelector(".statement-eep-text")?.textContent).toBe("Not available on this node");
  });

  it("handles CSV and JSON downloads with counterparty name filled", async () => {
    const mockStatement: Statement = {
      statement_version: 1,
      owner_did: "did:key:zMe",
      from_secs: 1700000000,
      to_secs: 1700086400,
      generated_at_secs: 1700086400,
      rows: [
        {
          agreement: "agr_1",
          commitment_id: "c1",
          agreed_at_secs: 1700001000,
          settled_at_secs: 1700005000,
          direction: "given",
          counterparty_did: "did:key:zBob",
          resource_kind: "service",
          commitment_kind: "fulfilment",
          description: { lang: "en", text: "Violin lesson" },
          currency: "INR",
          declared_value_minor: 60000,
          records: ["rec_1"],
        },
      ],
      open_at_end: [],
      totals: [],
      eeps: { status: "unavailable", reason: "Disabled in config" },
    };

    stubRpc({
      "statement.export": () => mockStatement,
      "contacts.list": () => [{ person_did: "did:key:zBob", display_name: "Bob Expert" }],
    });

    await renderStatement(host);
    const showBtn = host.querySelector<HTMLButtonElement>(".statement-show-button")!;
    showBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector(".statement-download-csv")).not.toBeNull();
    });

    const csvBtn = host.querySelector<HTMLButtonElement>(".statement-download-csv")!;
    csvBtn.click();

    expect(download).toHaveBeenCalledTimes(1);
    const [csvName, csvBlob] = (download as unknown as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(csvName).toBe("statement.csv");
    const csvContent = await readBlobText(csvBlob as Blob);
    expect(csvContent).toContain("agreement,commitment_id,agreed_at");
    expect(csvContent).toContain("Bob Expert");
    expect(csvContent).toContain("Violin lesson");

    const jsonBtn = host.querySelector<HTMLButtonElement>(".statement-download-json")!;
    jsonBtn.click();

    expect(download).toHaveBeenCalledTimes(2);
    const [jsonName, jsonBlob] = (download as unknown as ReturnType<typeof vi.fn>).mock.calls[1];
    expect(jsonName).toBe("statement.json");
    const jsonContent = await readBlobText(jsonBlob as Blob);
    const parsed = JSON.parse(jsonContent);
    expect(parsed.rows[0].counterparty_name).toBe("Bob Expert");

    // EEP unavailable line
    expect(host.querySelector(".statement-eep-text")?.textContent).toBe("Could not be read: Disabled in config");
  });
});
