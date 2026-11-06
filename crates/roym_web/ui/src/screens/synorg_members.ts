import { errText, field, input, refiller, text } from "../dom";
import { formatUtcDate } from "../i18n/format";
import { lookupText, messageId, t } from "../i18n/i18n";
import { call } from "../rpc";

/// One row a SynOrg holds about a record it signed: `credential.list`,
/// `revocation.list` and `member.decisions` all answer with these. The
/// signed envelope is the source of truth for what the record says.
interface IssuedRow {
  record_id: string;
  member_did: string;
  about: string;
  issued_at_secs: number;
  envelope: string;
}

interface CredentialView {
  row: IssuedRow;
  categories: string[];
  expiresAtSecs: number;
}

interface DecisionView {
  row: IssuedRow;
  action: "suspend" | "lift";
  /// The listing a suspension covers, or `undefined` for the whole membership.
  listingId?: string;
  rule: string;
  reason: string;
  untilSecs?: number;
}

function parseEnvelope(row: IssuedRow): { payload: Record<string, unknown>; expires?: number } {
  try {
    const env = JSON.parse(row.envelope) as { payload?: Record<string, unknown>; expires_at_secs?: number };
    return { payload: env.payload ?? {}, expires: env.expires_at_secs };
  } catch {
    return { payload: {} };
  }
}

function credentialView(row: IssuedRow): CredentialView {
  const { payload, expires } = parseEnvelope(row);
  const scope = payload.scope as { categories?: string[] } | undefined;
  return { row, categories: scope?.categories ?? [], expiresAtSecs: expires ?? 0 };
}

function decisionView(row: IssuedRow): DecisionView {
  const { payload } = parseEnvelope(row);
  const scope = payload.scope as { kind?: string; listing_id?: string } | undefined;
  return {
    row,
    action: payload.action === "lift" ? "lift" : "suspend",
    listingId: scope?.kind === "listing" ? String(scope.listing_id) : undefined,
    rule: String(payload.rule ?? ""),
    reason: String(payload.reason ?? ""),
    untilSecs: typeof payload.until_secs === "number" ? payload.until_secs : undefined,
  };
}

/// Message id for each credential status the Hub makes itself. The status
/// itself stays the code in `dataset.status`.
const CREDENTIAL_STATUS_LABELS = new Map<string, string>([
  ["revoked", messageId("synorg-credential-status-revoked")],
  ["current", messageId("synorg-credential-status-current")],
  ["replaced", messageId("synorg-credential-status-replaced")],
]);

function decisionWhat(v: DecisionView): string {
  if (v.action === "lift") return t("synorg-decision-lifted");
  return v.listingId === undefined
    ? t("synorg-decision-suspended-membership")
    : t("synorg-decision-suspended-listing", { id: v.listingId });
}

async function records(method: string): Promise<IssuedRow[]> {
  const res = await call<{ records: IssuedRow[] }>(method);
  return res.records ?? [];
}

/// Issue, revoke, suspend and lift, all from one panel of the SynOrg
/// screen. Every one of these is the group owner's own decision, signed on
/// this installation; nothing here reaches another installation, and the
/// notice below says so in the same words as everywhere else.
export async function buildMembersPanel(synorgCategories: string[]): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "synorg-members";
  wrap.appendChild(text("h3", t("synorg-members-heading")));
  wrap.appendChild(text("p", t("membership-no-instant-removal"), "no-instant-removal-notice"));

  const lists = document.createElement("div");
  lists.className = "members-lists";
  const reload = refiller(lists, async (into) => {
    try {
      const [credentials, revocations, decisions] = await Promise.all([
        records("credential.list"),
        records("revocation.list"),
        records("member.decisions"),
      ]);
      into.append(
        buildCredentialList(credentials, revocations, reload),
        buildDecisionList(decisions, reload),
      );
    } catch (err) {
      into.appendChild(text("p", t("synorg-members-load-failed", { detail: errText(err) })));
    }
  });

  wrap.append(buildIssueForm(synorgCategories, reload), buildSuspendForm(reload), lists);
  await reload();
  return wrap;
}

function buildIssueForm(categories: string[], reload: () => Promise<void>): HTMLElement {
  const form = document.createElement("div");
  form.className = "issue-credential-form";
  form.appendChild(text("h4", t("synorg-issue-heading")));
  form.appendChild(text("p", t("synorg-issue-help")));
  const member = input(t("synorg-member-did-placeholder"));
  member.className = "issue-member-did";
  const boxes = categories.map((c) => {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "issue-category";
    box.value = c;
    box.checked = true;
    const label = document.createElement("label");
    label.className = "issue-category-label";
    label.append(box, text("span", c));
    return { box, label };
  });
  const days = document.createElement("input");
  days.className = "issue-days";
  days.value = "365";
  const button = text("button", t("synorg-issue-button"), "button issue-credential") as HTMLButtonElement;
  const status = text("p", "", "issue-status");

  button.onclick = async () => {
    status.textContent = "";
    const chosen = boxes.filter((b) => b.box.checked).map((b) => b.box.value);
    const n = Number.parseInt(days.value, 10);
    if (!member.value.trim()) return void (status.textContent = t("synorg-issue-enter-did"));
    if (chosen.length === 0) return void (status.textContent = t("synorg-issue-choose-category"));
    if (!Number.isInteger(n) || n < 1 || n > 730) {
      return void (status.textContent = t("synorg-issue-days-range"));
    }
    button.disabled = true;
    try {
      await call("credential.issue", {
        member_did: member.value.trim(),
        categories: chosen,
        expires_at_secs: Math.floor(Date.now() / 1000) + n * 86400,
      });
      status.textContent = t("synorg-issued");
      member.value = "";
      await reload();
    } catch (err) {
      status.textContent = t("synorg-issue-failed", { detail: errText(err) });
    }
    button.disabled = false;
  };
  form.append(
    field(t("synorg-label-member-did"), member),
    text("div", t("synorg-covers-label"), "issue-categories-label"),
    ...boxes.map((b) => b.label),
    field(t("synorg-label-valid-days"), days),
    button,
    status,
  );
  return form;
}

function buildSuspendForm(reload: () => Promise<void>): HTMLElement {
  const form = document.createElement("div");
  form.className = "suspend-form";
  form.appendChild(text("h4", t("synorg-suspend-heading")));
  const member = input(t("synorg-member-did-placeholder"));
  member.className = "suspend-member-did";
  const listing = input(t("synorg-suspend-listing-placeholder"));
  listing.className = "suspend-listing-id";
  const rule = input(t("synorg-suspend-rule-placeholder"));
  rule.className = "suspend-rule";
  const reason = input(t("synorg-reason-placeholder"));
  reason.className = "suspend-reason";
  const days = input(t("synorg-suspend-days-placeholder"));
  days.className = "suspend-days";
  const button = text("button", t("synorg-suspend-button"), "button suspend-member") as HTMLButtonElement;
  const status = text("p", "", "suspend-status");

  button.onclick = async () => {
    status.textContent = "";
    if (!member.value.trim() || !rule.value.trim()) {
      status.textContent = t("synorg-suspend-required");
      return;
    }
    const params: Record<string, unknown> = {
      member_did: member.value.trim(),
      rule: rule.value.trim(),
      reason: reason.value.trim(),
    };
    if (listing.value.trim()) params.scope = { kind: "listing", listing_id: listing.value.trim() };
    if (days.value.trim()) {
      const n = Number.parseInt(days.value, 10);
      if (!Number.isInteger(n) || n < 1) {
        status.textContent = t("synorg-suspend-days-min");
        return;
      }
      params.until_secs = Math.floor(Date.now() / 1000) + n * 86400;
    }
    button.disabled = true;
    try {
      await call("member.suspend", params);
      status.textContent = t("synorg-suspended");
      await reload();
    } catch (err) {
      status.textContent = t("synorg-suspend-failed", { detail: errText(err) });
    }
    button.disabled = false;
  };
  form.append(
    field(t("synorg-label-member-did"), member),
    field(t("synorg-label-one-listing"), listing),
    field(t("synorg-label-rule"), rule),
    field(t("synorg-label-reason"), reason),
    field(t("synorg-label-how-long"), days),
    button,
    status,
  );
  return form;
}

function buildCredentialList(
  credentials: IssuedRow[],
  revocations: IssuedRow[],
  reload: () => Promise<void>,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "credential-list";
  wrap.appendChild(text("h4", t("synorg-credentials-heading")));
  const revoked = new Set(revocations.map((r) => r.about));
  const newestPerMember = new Map<string, IssuedRow>();
  for (const c of credentials) {
    const seen = newestPerMember.get(c.member_did);
    if (!seen || c.issued_at_secs >= seen.issued_at_secs) newestPerMember.set(c.member_did, c);
  }
  if (credentials.length === 0) wrap.appendChild(text("p", t("synorg-credentials-empty"), "credential-empty"));
  for (const row of credentials.slice().sort((a, b) => b.issued_at_secs - a.issued_at_secs)) {
    const view = credentialView(row);
    const isCurrent = newestPerMember.get(row.member_did)?.record_id === row.record_id;
    const status = revoked.has(row.record_id) ? "revoked" : isCurrent ? "current" : "replaced";
    const line = document.createElement("div");
    line.className = "credential-row";
    line.dataset.status = status;
    line.dataset.member = row.member_did;
    line.appendChild(text("span", row.member_did, "credential-member"));
    line.appendChild(text("span", t("synorg-credential-covers", { categories: view.categories.join(", ") }), "credential-covers"));
    line.appendChild(
      text("span", t("synorg-credential-expires", { date: formatUtcDate(view.expiresAtSecs) }), "credential-expires"),
    );
    line.appendChild(
      text("span", lookupText(CREDENTIAL_STATUS_LABELS, status) ?? status, "credential-status"),
    );
    if (status === "current") line.appendChild(revokeControl(row, reload));
    wrap.appendChild(line);
  }
  return wrap;
}

function revokeControl(row: IssuedRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "revoke-control";
  const reason = input(t("synorg-reason-placeholder"));
  reason.className = "revoke-reason";
  const button = text("button", t("synorg-revoke-button"), "button revoke-credential") as HTMLButtonElement;
  const status = text("span", "", "revoke-status");
  button.onclick = async () => {
    button.disabled = true;
    status.textContent = "";
    try {
      await call("revocation.issue", {
        credential_record_id: row.record_id,
        reason: reason.value.trim(),
      });
      await reload();
    } catch (err) {
      status.textContent = t("synorg-revoke-failed", { detail: errText(err) });
      button.disabled = false;
    }
  };
  wrap.append(reason, button, status);
  return wrap;
}

function buildDecisionList(decisions: IssuedRow[], reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "decision-list";
  wrap.appendChild(text("h4", t("synorg-decisions-heading")));
  const views = decisions.map(decisionView).sort((a, b) => b.row.issued_at_secs - a.row.issued_at_secs);
  const lifted = new Set(views.filter((v) => v.action === "lift").map((v) => v.row.about));
  if (views.length === 0) wrap.appendChild(text("p", t("synorg-decisions-empty"), "decision-empty"));
  for (const v of views) {
    const line = document.createElement("div");
    line.className = "decision-row";
    const active = v.action === "suspend" && !lifted.has(v.row.record_id);
    line.dataset.status = v.action === "lift" ? "lift" : active ? "active" : "lifted";
    line.appendChild(text("span", v.row.member_did, "decision-member"));
    line.appendChild(text("span", decisionWhat(v), "decision-what"));
    if (v.action === "suspend") {
      line.appendChild(text("span", t("synorg-decision-rule", { rule: v.rule }), "decision-rule"));
      const until = v.untilSecs
        ? t("synorg-until", { date: formatUtcDate(v.untilSecs) })
        : t("synorg-until-lifted");
      line.appendChild(text("span", until, "decision-until"));
    }
    line.appendChild(text("span", formatUtcDate(v.row.issued_at_secs), "decision-date"));
    if (active) line.appendChild(liftControl(v.row, reload));
    wrap.appendChild(line);
  }
  return wrap;
}

function liftControl(row: IssuedRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "lift-control";
  const button = text("button", t("synorg-lift-button"), "button lift-suspension") as HTMLButtonElement;
  const status = text("span", "", "lift-status");
  button.onclick = async () => {
    button.disabled = true;
    status.textContent = "";
    try {
      await call("member.lift", { decision_record_id: row.record_id });
      await reload();
    } catch (err) {
      status.textContent = t("synorg-lift-failed", { detail: errText(err) });
      button.disabled = false;
    }
  };
  wrap.append(button, status);
  return wrap;
}
