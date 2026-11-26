import { describe, expect, it } from "vitest";
import { formatLocalIso, guardFormula, quoteCsvField, statementToCsv } from "./statement_csv";

describe("statement CSV writer", () => {
  it("guards against formula execution across all text columns", () => {
    expect(guardFormula("=HYPERLINK()")).toBe("'=HYPERLINK()");
    expect(guardFormula("+447123456")).toBe("'+447123456");
    expect(guardFormula("-10")).toBe("'-10");
    expect(guardFormula("@admin")).toBe("'@admin");
    expect(guardFormula("\tname")).toBe("'\tname");
    expect(guardFormula("\rname")).toBe("'\rname");
    expect(guardFormula("safe text")).toBe("safe text");
  });

  it("quotes RFC 4180 fields correctly", () => {
    expect(quoteCsvField("simple")).toBe("simple");
    expect(quoteCsvField("with,comma")).toBe('"with,comma"');
    expect(quoteCsvField('with"quote')).toBe('"with""quote"');
    expect(quoteCsvField("with\nnewline")).toBe('"with\nnewline"');
  });

  it("formats local offset ISO timestamp", () => {
    const formatted = formatLocalIso(1700000000);
    // Regex for YYYY-MM-DDTHH:mm:ss[+-]HH:mm
    expect(formatted).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
  });

  it("exports statement rows with decimal declared_value and guarded counterparty_name", () => {
    const rows = [
      {
        agreement: "agr_1",
        commitment_id: "c1",
        agreed_at_secs: 1700000000,
        settled_at_secs: 1700003600,
        direction: "given",
        counterparty_did: "did:key:zCounterparty",
        counterparty_name: "=HYPERLINK('http://evil.com', 'Bob')", // formula attempt
        resource_kind: "goods-or-service",
        commitment_kind: "fulfilment",
        description: { lang: "en", text: "Piano lesson, 1 hour" },
        currency: "USD",
        declared_value_minor: 5000, // 50.00
        settlement_note: "+Handed over in person", // formula attempt
      },
    ];

    const csv = statementToCsv(rows);
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe(
      "agreement,commitment_id,agreed_at,settled_at,direction,counterparty_did,counterparty_name,resource_kind,commitment_kind,description,currency,declared_value,settlement_note",
    );

    const rowLine = lines[1];
    // counterparty_name must be guarded with leading single-quote and quoted due to comma
    expect(rowLine).toContain("\"'=HYPERLINK('http://evil.com', 'Bob')\"");
    // description with comma must be quoted
    expect(rowLine).toContain('"Piano lesson, 1 hour"');
    // decimal value must be 50.00 without quotes
    expect(rowLine).toContain(",50.00,");
    // settlement note must be guarded with leading single-quote
    expect(rowLine).toContain("'+Handed over in person");
  });
});
