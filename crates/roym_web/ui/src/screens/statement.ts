import { download } from "../download";
import { errText, text } from "../dom";
import { formatMinor } from "../money";
import { call } from "../rpc";
import { t } from "../i18n/i18n";
import { statementToCsv, type StatementRowData } from "../statement_csv";

export interface StatementRow {
  agreement: string;
  commitment_id: string;
  agreed_at_secs: number;
  settled_at_secs: number;
  direction: "given" | "received";
  counterparty_did: string;
  counterparty_name?: string;
  resource_kind: string;
  commitment_kind: string;
  description: { lang?: string; text?: string } | string;
  currency: string;
  declared_value_minor: number | bigint;
  records: string[];
  settlement_note?: { lang?: string; text?: string } | string;
}

export interface StatementOpenAtEnd {
  agreement: string;
  commitment_id: string;
  agreed_at_secs: number;
  direction: "given" | "received";
  counterparty_did: string;
  counterparty_name?: string;
  resource_kind: string;
  commitment_kind: string;
  description: { lang?: string; text?: string } | string;
  currency: string;
  declared_value_minor: number | bigint;
}

export interface StatementTotal {
  currency: string;
  direction: "given" | "received";
  resource_kind: string;
  sum_minor: number | bigint;
}

export interface Statement {
  statement_version: number;
  owner_did: string;
  from_secs: number;
  to_secs: number;
  generated_at_secs: number;
  rows: StatementRow[];
  open_at_end: StatementOpenAtEnd[];
  totals: StatementTotal[];
  eeps: { status: "not-available" | "available" | "unavailable"; reason?: string };
}

export function toDateInputValue(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

export function formatLocalIso(secs: number): string {
  const d = new Date(secs * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const offMinutes = -d.getTimezoneOffset();
  const sign = offMinutes >= 0 ? "+" : "-";
  const absOff = Math.abs(offMinutes);
  const offH = pad(Math.floor(absOff / 60));
  const offM = pad(absOff % 60);

  const Y = d.getFullYear();
  const M = pad(d.getMonth() + 1);
  const D = pad(d.getDate());
  const h = pad(d.getHours());
  const m = pad(d.getMinutes());
  const s = pad(d.getSeconds());

  return `${Y}-${M}-${D}T${h}:${m}:${s}${sign}${offH}:${offM}`;
}

export function parseDateInput(val: string): Date | null {
  const parts = val.split("-").map(Number);
  if (parts.length !== 3 || parts.some(isNaN)) return null;
  return new Date(parts[0], parts[1] - 1, parts[2], 0, 0, 0, 0);
}

export function thisMonthRange(now = new Date()): { fromStr: string; toStr: string; from_secs: number; to_secs: number } {
  const start = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), now.getMonth() + 1, 0, 0, 0, 0);
  const nextMonthStart = new Date(now.getFullYear(), now.getMonth() + 1, 1, 0, 0, 0, 0);
  return {
    fromStr: toDateInputValue(start),
    toStr: toDateInputValue(end),
    from_secs: Math.floor(start.getTime() / 1000),
    to_secs: Math.floor(nextMonthStart.getTime() / 1000),
  };
}

export function lastMonthRange(now = new Date()): { fromStr: string; toStr: string; from_secs: number; to_secs: number } {
  const start = new Date(now.getFullYear(), now.getMonth() - 1, 1, 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), now.getMonth(), 0, 0, 0, 0);
  const thisMonthStart = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
  return {
    fromStr: toDateInputValue(start),
    toStr: toDateInputValue(end),
    from_secs: Math.floor(start.getTime() / 1000),
    to_secs: Math.floor(thisMonthStart.getTime() / 1000),
  };
}

export function thisYearRange(now = new Date()): { fromStr: string; toStr: string; from_secs: number; to_secs: number } {
  const start = new Date(now.getFullYear(), 0, 1, 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), 11, 31, 0, 0, 0, 0);
  const nextYearStart = new Date(now.getFullYear() + 1, 0, 1, 0, 0, 0, 0);
  return {
    fromStr: toDateInputValue(start),
    toStr: toDateInputValue(end),
    from_secs: Math.floor(start.getTime() / 1000),
    to_secs: Math.floor(nextYearStart.getTime() / 1000),
  };
}

function textOf(val: unknown): string {
  if (typeof val === "string") return val;
  if (typeof val === "object" && val !== null && "text" in val) {
    return String((val as { text?: unknown }).text ?? "");
  }
  return "";
}

function resourceKindText(kind: string): string {
  switch (kind) {
    case "service":
      return t("deal-kind-service");
    case "goods":
      return t("deal-kind-goods");
    case "currency":
      return t("deal-kind-money");
    case "goods-or-service":
      return t("statement-kind-goods-or-service");
    default:
      return kind;
  }
}

function renderStatementDownloads(statement: Statement): HTMLElement {
  const actions = document.createElement("div");
  actions.className = "statement-actions";

  const downloadCsvBtn = text("button", t("statement-download-csv"), "button statement-download-csv") as HTMLButtonElement;
  downloadCsvBtn.onclick = () => {
    const csvStr = statementToCsv(statement.rows as StatementRowData[]);
    const blob = new Blob([csvStr], { type: "text/csv;charset=utf-8;" });
    download("statement.csv", blob);
  };

  const downloadJsonBtn = text("button", t("statement-download-json"), "button statement-download-json") as HTMLButtonElement;
  downloadJsonBtn.onclick = () => {
    const jsonStr = JSON.stringify(statement, null, 2);
    const blob = new Blob([jsonStr], { type: "application/json" });
    download("statement.json", blob);
  };

  actions.appendChild(downloadCsvBtn);
  actions.appendChild(downloadJsonBtn);
  return actions;
}

function renderStatementRows(statement: Statement): HTMLElement {
  const rowsSection = document.createElement("div");
  rowsSection.className = "statement-settled-section";
  if (statement.rows.length === 0) {
    rowsSection.appendChild(text("p", t("statement-no-rows"), "statement-empty"));
    return rowsSection;
  }

  const rowsList = document.createElement("div");
  rowsList.className = "statement-rows";
  for (const row of statement.rows) {
    const rowEl = document.createElement("div");
    rowEl.className = "statement-row";

    const dirText = row.direction === "given" ? t("statement-direction-given") : t("statement-direction-received");
    const cpText = row.counterparty_name || row.counterparty_did;
    const valText = formatMinor(row.declared_value_minor, row.currency);
    const descText = textOf(row.description);
    const dateText = formatLocalIso(row.settled_at_secs);

    rowEl.appendChild(text("span", cpText, "statement-row-counterparty"));
    rowEl.appendChild(text("span", descText, "statement-row-description"));
    rowEl.appendChild(text("span", dirText, "statement-row-direction"));
    rowEl.appendChild(text("span", valText, "statement-row-value"));
    rowEl.appendChild(text("span", dateText, "statement-row-dates"));

    if (row.settlement_note) {
      const noteText = textOf(row.settlement_note);
      if (noteText) {
        rowEl.appendChild(text("span", noteText, "statement-row-note"));
      }
    }
    rowsList.appendChild(rowEl);
  }
  rowsSection.appendChild(rowsList);
  return rowsSection;
}

function renderStatementOpenAtEnd(statement: Statement): HTMLElement {
  const openSection = document.createElement("div");
  openSection.className = "statement-open-section";
  openSection.appendChild(text("h3", t("statement-open-heading"), "statement-open-heading"));

  if (statement.open_at_end.length === 0) {
    openSection.appendChild(text("p", t("statement-open-none"), "statement-open-none"));
    return openSection;
  }

  const openRows = document.createElement("div");
  openRows.className = "statement-open-rows";
  for (const row of statement.open_at_end) {
    const rowEl = document.createElement("div");
    rowEl.className = "statement-open-row";

    const dirText = row.direction === "given" ? t("statement-direction-given") : t("statement-direction-received");
    const cpText = row.counterparty_name || row.counterparty_did;
    const valText = formatMinor(row.declared_value_minor, row.currency);
    const descText = textOf(row.description);

    rowEl.appendChild(text("span", cpText, "statement-row-counterparty"));
    rowEl.appendChild(text("span", descText, "statement-row-description"));
    rowEl.appendChild(text("span", dirText, "statement-row-direction"));
    rowEl.appendChild(text("span", valText, "statement-row-value"));

    openRows.appendChild(rowEl);
  }
  openSection.appendChild(openRows);
  return openSection;
}

function renderStatementTotals(statement: Statement): HTMLElement {
  const totalsSection = document.createElement("div");
  totalsSection.className = "statement-totals";
  totalsSection.appendChild(text("h3", t("statement-totals-heading"), "statement-totals-heading"));

  if (statement.totals.length === 0) {
    totalsSection.appendChild(text("p", t("statement-totals-none"), "statement-totals-none"));
    return totalsSection;
  }

  const totalsList = document.createElement("div");
  totalsList.className = "statement-totals-rows";
  for (const total of statement.totals) {
    const totalEl = document.createElement("div");
    totalEl.className = "statement-total-row";

    const dirText = total.direction === "given" ? t("statement-direction-given") : t("statement-direction-received");
    const valText = formatMinor(total.sum_minor, total.currency);
    const kindText = resourceKindText(total.resource_kind);

    totalEl.appendChild(text("span", dirText, "statement-total-direction"));
    totalEl.appendChild(text("span", kindText, "statement-total-kind"));
    totalEl.appendChild(text("span", valText, "statement-total-sum"));

    totalsList.appendChild(totalEl);
  }
  totalsSection.appendChild(totalsList);
  return totalsSection;
}

function renderStatementEep(statement: Statement): HTMLElement {
  const eepSection = document.createElement("div");
  eepSection.className = "statement-eep-line";
  if (statement.eeps.status === "not-available") {
    eepSection.appendChild(text("p", t("statement-eep-not-available"), "statement-eep-text"));
  } else if (statement.eeps.status === "unavailable") {
    eepSection.appendChild(text("p", t("statement-eep-unavailable", { reason: statement.eeps.reason || "" }), "statement-eep-text"));
  } else if (statement.eeps.status === "available") {
    eepSection.appendChild(text("p", t("statement-eep-available"), "statement-eep-text"));
  }
  return eepSection;
}

async function handleStatementShow(
  fromInput: HTMLInputElement,
  toInput: HTMLInputElement,
  errorEl: HTMLElement,
  resultsEl: HTMLElement,
) {
  errorEl.textContent = "";
  resultsEl.replaceChildren();

  const fromDate = parseDateInput(fromInput.value);
  const toDate = parseDateInput(toInput.value);

  if (!fromDate || !toDate) {
    errorEl.textContent = t("statement-error-period");
    return;
  }

  if (fromDate.getTime() > toDate.getTime()) {
    errorEl.textContent = t("statement-error-period");
    return;
  }

  const from_secs = Math.floor(fromDate.getTime() / 1000);
  const toDatePlusOne = new Date(toDate.getFullYear(), toDate.getMonth(), toDate.getDate() + 1, 0, 0, 0, 0);
  const to_secs = Math.floor(toDatePlusOne.getTime() / 1000);

  let statement: Statement;
  let contacts: Array<{ person_did: string; display_name?: string }> = [];

  try {
    const [sRes, cRes] = await Promise.all([
      call<Statement>("statement.export", { from_secs, to_secs }),
      call<Array<{ person_did: string; display_name?: string }>>("contacts.list").catch(() => []),
    ]);
    statement = sRes;
    contacts = cRes;
  } catch (err) {
    errorEl.textContent = t("statement-fetch-failed", { detail: errText(err) });
    return;
  }

  const nameMap = new Map<string, string>();
  for (const c of contacts) {
    if (c.display_name) nameMap.set(c.person_did, c.display_name);
  }
  for (const row of statement.rows) {
    if (!row.counterparty_name && nameMap.has(row.counterparty_did)) {
      row.counterparty_name = nameMap.get(row.counterparty_did);
    }
  }
  for (const row of statement.open_at_end) {
    if (!row.counterparty_name && nameMap.has(row.counterparty_did)) {
      row.counterparty_name = nameMap.get(row.counterparty_did);
    }
  }

  resultsEl.append(
    renderStatementDownloads(statement),
    renderStatementRows(statement),
    renderStatementOpenAtEnd(statement),
    renderStatementTotals(statement),
    renderStatementEep(statement),
  );
}

export async function renderStatement(container: HTMLElement): Promise<void> {
  container.replaceChildren();

  const screen = document.createElement("div");
  screen.className = "statement-screen";
  screen.appendChild(text("h2", t("statement-title")));

  const controls = document.createElement("div");
  controls.className = "statement-period-controls";

  const periodButtons = document.createElement("div");
  periodButtons.className = "statement-period-buttons";

  const thisMonthBtn = text("button", t("statement-period-this-month"), "button statement-period-this-month") as HTMLButtonElement;
  const lastMonthBtn = text("button", t("statement-period-last-month"), "button statement-period-last-month") as HTMLButtonElement;
  const thisYearBtn = text("button", t("statement-period-this-year"), "button statement-period-this-year") as HTMLButtonElement;

  periodButtons.append(thisMonthBtn, lastMonthBtn, thisYearBtn);
  controls.appendChild(periodButtons);

  const datesRow = document.createElement("div");
  datesRow.className = "statement-dates-row";

  const fromLabel = document.createElement("label");
  fromLabel.className = "statement-from-label";
  fromLabel.textContent = t("statement-from-label");
  const fromInput = document.createElement("input");
  fromInput.type = "date";
  fromInput.className = "statement-from-input";
  fromLabel.appendChild(fromInput);

  const toLabel = document.createElement("label");
  toLabel.className = "statement-to-label";
  toLabel.textContent = t("statement-to-label");
  const toInput = document.createElement("input");
  toInput.type = "date";
  toInput.className = "statement-to-input";
  toLabel.appendChild(toInput);

  datesRow.append(fromLabel, toLabel);
  controls.appendChild(datesRow);

  const initialRange = thisMonthRange();
  fromInput.value = initialRange.fromStr;
  toInput.value = initialRange.toStr;

  thisMonthBtn.onclick = () => {
    const range = thisMonthRange();
    fromInput.value = range.fromStr;
    toInput.value = range.toStr;
  };
  lastMonthBtn.onclick = () => {
    const range = lastMonthRange();
    fromInput.value = range.fromStr;
    toInput.value = range.toStr;
  };
  thisYearBtn.onclick = () => {
    const range = thisYearRange();
    fromInput.value = range.fromStr;
    toInput.value = range.toStr;
  };

  const showBtn = text("button", t("statement-show"), "button statement-show-button") as HTMLButtonElement;
  controls.appendChild(showBtn);
  screen.appendChild(controls);

  const errorEl = document.createElement("p");
  errorEl.className = "statement-error";
  screen.appendChild(errorEl);

  const resultsEl = document.createElement("div");
  resultsEl.className = "statement-results";
  screen.appendChild(resultsEl);

  container.appendChild(screen);

  showBtn.onclick = () => handleStatementShow(fromInput, toInput, errorEl, resultsEl);
}
