import { currencyMinorExponent, minorToDecimal } from "./money";

export interface StatementRowData {
  agreement: string;
  commitment_id: string;
  agreed_at_secs: number;
  settled_at_secs: number;
  direction: string;
  counterparty_did: string;
  counterparty_name?: string;
  resource_kind: string;
  commitment_kind: string;
  description: { lang?: string; text?: string } | string;
  currency: string;
  declared_value_minor: number | bigint;
  settlement_note?: { lang?: string; text?: string } | string;
  records?: string[];
  [key: string]: unknown;
}

const FORMULA_PREFIXES = ["=", "+", "-", "@", "\t", "\r"];

/**
 * Applies CSV formula guard: if a text cell begins with =, +, -, @, tab or CR,
 * prepend a single quote (') so spreadsheets do not execute it as a formula.
 */
export function guardFormula(val: string): string {
  if (val.length > 0 && FORMULA_PREFIXES.some((p) => val.startsWith(p))) {
    return `'${val}`;
  }
  return val;
}

/** Quotes a field according to RFC 4180 if it contains comma, quote, or newline. */
export function quoteCsvField(val: string): string {
  if (val.includes(",") || val.includes('"') || val.includes("\n") || val.includes("\r")) {
    return `"${val.replace(/"/g, '""')}"`;
  }
  return val;
}

/** Formats timestamp seconds into ISO 8601 with local timezone offset. */
export function formatLocalIso(secs: number): string {
  const d = new Date(secs * 1000);
  const Y = d.getFullYear();
  const M = String(d.getMonth() + 1).padStart(2, "0");
  const D = String(d.getDate()).padStart(2, "0");
  const h = String(d.getHours()).padStart(2, "0");
  const m = String(d.getMinutes()).padStart(2, "0");
  const s = String(d.getSeconds()).padStart(2, "0");

  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const absOff = Math.abs(offset);
  const offH = String(Math.floor(absOff / 60)).padStart(2, "0");
  const offM = String(absOff % 60).padStart(2, "0");

  return `${Y}-${M}-${D}T${h}:${m}:${s}${sign}${offH}:${offM}`;
}

function textOf(val: unknown): string {
  if (typeof val === "string") return val;
  if (typeof val === "object" && val !== null && "text" in val) {
    return String((val as { text?: unknown }).text ?? "");
  }
  return "";
}

export const STATEMENT_CSV_HEADER = [
  "agreement",
  "commitment_id",
  "agreed_at",
  "settled_at",
  "direction",
  "counterparty_did",
  "counterparty_name",
  "resource_kind",
  "commitment_kind",
  "description",
  "currency",
  "declared_value",
  "settlement_note",
].join(",");

/**
 * Pure function that serializes Statement rows into RFC 4180 CSV with formula guard.
 */
export function statementToCsv(rows: StatementRowData[]): string {
  const lines = [STATEMENT_CSV_HEADER];

  for (const row of rows) {
    const exp = currencyMinorExponent(row.currency) ?? 2;
    const decimalValue = minorToDecimal(row.declared_value_minor, exp);

    const fields = [
      quoteCsvField(guardFormula(row.agreement)),
      quoteCsvField(guardFormula(row.commitment_id)),
      quoteCsvField(guardFormula(formatLocalIso(row.agreed_at_secs))),
      quoteCsvField(guardFormula(formatLocalIso(row.settled_at_secs))),
      quoteCsvField(guardFormula(row.direction)),
      quoteCsvField(guardFormula(row.counterparty_did)),
      quoteCsvField(guardFormula(row.counterparty_name || "")),
      quoteCsvField(guardFormula(row.resource_kind)),
      quoteCsvField(guardFormula(row.commitment_kind)),
      quoteCsvField(guardFormula(textOf(row.description))),
      quoteCsvField(guardFormula(row.currency)),
      quoteCsvField(decimalValue),
      quoteCsvField(guardFormula(textOf(row.settlement_note))),
    ];

    lines.push(fields.join(","));
  }

  return lines.join("\r\n") + "\r\n";
}
