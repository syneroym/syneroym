import { text } from "../dom";
import { formatUtcDate } from "../i18n/format";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import { openForwardPicker } from "./forward_picker";

/** The content type of a message that forwards a signed Roym record. */
export const RECORD_FORWARD_CONTENT_TYPE = "application/vnd.roym.record-forward+json";

export type RecordForwardType = "listing" | "profile" | "quote";

const RECORD_FORWARD_TYPES: RecordForwardType[] = ["listing", "profile", "quote"];

export interface RecordForwardBody {
  record_forward_version: number;
  record_type: string;
  envelope: string;
}

interface Verdict {
  verified?: boolean;
  reason?: string;
  issuer?: string;
  payload?: Record<string, unknown>;
}

/** Reads the body of a record-forward message. `null` means it is not one
 * this build shows; the screen names that instead of drawing raw JSON. */
export function parseRecordForward(body: string | undefined): RecordForwardBody | null {
  if (!body) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const v = parsed as RecordForwardBody;
  if (v.record_forward_version !== 1) return null;
  if (!isRecordForwardType(v.record_type)) return null;
  if (typeof v.envelope !== "string" || v.envelope.length === 0) return null;
  return v;
}

export function isRecordForwardType(value: string | undefined): value is RecordForwardType {
  return value !== undefined && (RECORD_FORWARD_TYPES as string[]).includes(value);
}

/** The service that owns a record type is the one that checks it. */
async function verifyRecord(type: RecordForwardType, envelope: string): Promise<Verdict> {
  switch (type) {
    case "listing":
      return call<Verdict>("listing.verify", { envelope });
    case "profile":
      return call<Verdict>("profile.verify-envelope", { envelope });
    case "quote":
      return call<Verdict>("quote.verify", { envelope, disclosures: [] });
  }
}

function headingFor(type: RecordForwardType): string {
  switch (type) {
    case "listing":
      return t("record-forward-listing-heading");
    case "profile":
      return t("record-forward-profile-heading");
    case "quote":
      return t("record-forward-quote-heading");
  }
}

function stringField(payload: Record<string, unknown> | undefined, key: string): string {
  const value = payload?.[key];
  return typeof value === "string" ? value : "";
}

function stringList(value: unknown): string {
  return Array.isArray(value) ? value.filter((v) => typeof v === "string").join(", ") : "";
}

function addLine(box: HTMLElement, className: string, value: string): void {
  if (!value) return;
  box.appendChild(text("p", value, className));
}

function renderListing(box: HTMLElement, payload: Record<string, unknown> | undefined): void {
  addLine(box, "record-forward-title", stringField(payload, "title"));
  addLine(box, "record-forward-summary", stringField(payload, "summary"));
  const categories = stringList(payload?.["categories"]);
  if (categories) {
    box.appendChild(
      text("p", t("record-forward-categories", { categories }), "record-forward-categories"),
    );
  }
  addLine(box, "record-forward-status-line", stringField(payload, "status"));
}

function renderProfile(box: HTMLElement, payload: Record<string, unknown> | undefined): void {
  addLine(box, "record-forward-name", stringField(payload, "display_name"));
  addLine(box, "record-forward-about", stringField(payload, "about"));
}

interface Commitment {
  commitment_id?: unknown;
  giver?: unknown;
  resource_kind?: unknown;
}

function giverDid(giver: string, issuer: string, counterparty: string): string {
  switch (giver) {
    case "quote-author":
      return issuer;
    case "counterparty":
      return counterparty;
    default:
      return giver;
  }
}

function resourceKindText(kind: string): string {
  switch (kind) {
    case "service-in-slot":
      return t("deal-kind-service");
    case "goods-or-service":
      return t("statement-kind-goods-or-service");
    case "currency":
      return t("deal-kind-money");
    default:
      return kind;
  }
}

function renderQuote(box: HTMLElement, verdict: Verdict): void {
  const payload = verdict.payload;
  const issuer = verdict.issuer ?? "";
  const counterparty = stringField(payload, "counterparty_did");
  if (issuer && counterparty) {
    box.appendChild(
      text(
        "p",
        t("record-forward-quote-parties", { author: issuer, counterparty }),
        "record-forward-quote-parties",
      ),
    );
  }
  const terms = payload?.["terms"];
  if (typeof terms !== "object" || terms === null) return;
  const signed = terms as Record<string, unknown>;
  addLine(
    box,
    "record-forward-quote-currency",
    stringField(signed, "currency")
      ? t("record-forward-quote-currency", { currency: stringField(signed, "currency") })
      : "",
  );
  const expires = signed["quote_expires_at_secs"];
  if (typeof expires === "number") {
    box.appendChild(
      text(
        "p",
        t("record-forward-quote-expiry", { date: formatUtcDate(expires) }),
        "record-forward-quote-expiry",
      ),
    );
  }
  const commitments = signed["commitments"];
  if (Array.isArray(commitments)) {
    for (const raw of commitments) {
      if (typeof raw !== "object" || raw === null) continue;
      const c = raw as Commitment;
      const line = t("record-forward-quote-commitment", {
        id: String(c.commitment_id ?? ""),
        giver: giverDid(String(c.giver ?? ""), issuer, counterparty),
        kind: resourceKindText(String(c.resource_kind ?? "")),
      });
      box.appendChild(text("p", line, "record-forward-quote-commitment"));
    }
  }
  box.appendChild(
    text("p", t("record-forward-quote-hidden"), "record-forward-quote-hidden"),
  );
}

function renderVerified(
  box: HTMLElement,
  type: RecordForwardType,
  verdict: Verdict,
): void {
  switch (type) {
    case "listing":
      renderListing(box, verdict.payload);
      break;
    case "profile":
      renderProfile(box, verdict.payload);
      break;
    case "quote":
      renderQuote(box, verdict);
      break;
  }
}

/** The block a received record-forward row shows. Every value is a text
 * node; a record that does not verify draws no field and no author. */
export function recordForwardBlock(body: RecordForwardBody): HTMLElement {
  const box = document.createElement("div");
  box.className = `record-forward-card record-forward-${body.record_type}`;
  box.appendChild(text("h3", headingFor(body.record_type as RecordForwardType), "record-forward-heading"));
  const type = body.record_type as RecordForwardType;
  void (async () => {
    try {
      const verdict = await verifyRecord(type, body.envelope);
      if (!verdict.verified) {
        appendNotChecked(box, verdict.reason);
        return;
      }
      renderVerified(box, type, verdict);
    } catch {
      appendNotChecked(box);
    }
  })();
  return box;
}

function appendNotChecked(host: HTMLElement, reason?: string): void {
  const note = text("p", t("record-forward-not-checked"), "record-forward-not-checked");
  if (reason) note.title = reason;
  host.appendChild(note);
}

async function pickAndForward(
  host: HTMLElement,
  status: HTMLElement,
  type: RecordForwardType,
  envelope: string,
  onSent?: () => void | Promise<void>,
): Promise<void> {
  await openForwardPicker(host, async (conversationId) => {
    await call("conversation.forward-record", {
      conversation: conversationId,
      record_type: type,
      envelope,
    });
    status.textContent = t("record-forward-sent");
    await onSent?.();
  });
}

/** Checks the record on this node, warns for a quote, then offers the
 * picker. A record that does not check is never offered for sending. */
export async function openRecordForward(
  host: HTMLElement,
  type: RecordForwardType,
  envelope: string,
  onSent?: () => void | Promise<void>,
  statusHost?: HTMLElement,
): Promise<void> {
  host.querySelector(".record-forward-status")?.remove();
  statusHost?.querySelector(".record-forward-status")?.remove();
  host.querySelector(".record-forward-warning")?.remove();
  host.querySelector(".forward-picker")?.remove();
  const status = text("p", "", "record-forward-status");
  (statusHost ?? host).appendChild(status);

  let verdict: Verdict;
  try {
    verdict = await verifyRecord(type, envelope);
  } catch {
    appendNotChecked(status);
    return;
  }
  if (!verdict.verified) {
    appendNotChecked(status, verdict.reason);
    return;
  }
  if (type === "quote") {
    const warning = document.createElement("div");
    warning.className = "record-forward-warning";
    warning.appendChild(text("p", t("record-forward-quote-warning")));
    const proceed = text(
      "button",
      t("record-forward-continue"),
      "button record-forward-continue",
    ) as HTMLButtonElement;
    proceed.onclick = () => {
      warning.remove();
      void pickAndForward(host, status, type, envelope, onSent);
    };
    const cancel = text(
      "button",
      t("common-cancel"),
      "button record-forward-cancel",
    ) as HTMLButtonElement;
    cancel.onclick = () => warning.remove();
    warning.append(proceed, cancel);
    host.appendChild(warning);
    return;
  }
  await pickAndForward(host, status, type, envelope, onSent);
}

export interface RecordForwardActionOptions {
  type: RecordForwardType;
  loadEnvelope: () => Promise<string | null>;
  onSent?: () => void | Promise<void>;
  /** A place outside the redrawn row for the "Forwarded." confirmation. */
  statusHost?: HTMLElement;
}

/** The "Forward" button for a quote card, a directory hit or a profile. */
export function recordForwardAction(options: RecordForwardActionOptions): HTMLElement {
  const box = document.createElement("div");
  box.className = "record-forward-actions";
  const button = text(
    "button",
    t("forward-button"),
    "button forward-button record-forward-button",
  ) as HTMLButtonElement;
  button.onclick = async () => {
    button.disabled = true;
    try {
      const envelope = await options.loadEnvelope();
      if (!envelope) {
        box.appendChild(text("p", t("record-forward-no-envelope"), "record-forward-status"));
        return;
      }
      await openRecordForward(box, options.type, envelope, options.onSent, options.statusHost);
    } finally {
      button.disabled = false;
    }
  };
  box.appendChild(button);
  return box;
}

/** The envelope of a quote card, matched by its record id. */
export async function quoteEnvelope(recordId: string | undefined): Promise<string | null> {
  if (!recordId) return null;
  const res = await call<{ quotes: Array<{ record_id: string; envelope: string }> }>(
    "quote.list",
    {},
  );
  return res.quotes?.find((q) => q.record_id === recordId)?.envelope ?? null;
}

/** The envelope of a stored profile of `personDid`, if this node has one. */
export async function profileEnvelope(personDid: string): Promise<string | null> {
  const res = await call<{ envelope?: string } | null>("profile.get", { person_did: personDid });
  return res?.envelope ?? null;
}
