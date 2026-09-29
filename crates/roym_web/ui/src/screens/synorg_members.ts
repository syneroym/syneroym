import { NO_INSTANT_REMOVAL_NOTICE } from "../directory/membership";
import { dateWords } from "../directory/words";
import { errText, field, text } from "../dom";
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
  scopeWords: string;
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
    scopeWords: scope?.kind === "listing" ? `one listing (${scope.listing_id})` : "the whole membership",
    rule: String(payload.rule ?? ""),
    reason: String(payload.reason ?? ""),
    untilSecs: typeof payload.until_secs === "number" ? payload.until_secs : undefined,
  };
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
  wrap.appendChild(text("h3", "Memberships and decisions"));
  wrap.appendChild(text("p", NO_INSTANT_REMOVAL_NOTICE, "no-instant-removal-notice"));

  const lists = document.createElement("div");
  lists.className = "members-lists";
  const reload = async () => {
    lists.replaceChildren();
    try {
      const [credentials, revocations, decisions] = await Promise.all([
        records("credential.list"),
        records("revocation.list"),
        records("member.decisions"),
      ]);
      lists.append(
        buildCredentialList(credentials, revocations, reload),
        buildDecisionList(decisions, reload),
      );
    } catch (err) {
      lists.appendChild(text("p", `Could not load memberships: ${errText(err)}`));
    }
  };

  wrap.append(buildIssueForm(synorgCategories, reload), buildSuspendForm(reload), lists);
  await reload();
  return wrap;
}

function buildIssueForm(categories: string[], reload: () => Promise<void>): HTMLElement {
  const form = document.createElement("div");
  form.className = "issue-credential-form";
  form.appendChild(text("h4", "Issue a membership credential"));
  form.appendChild(
    text(
      "p",
      "A provider gives you their DID outside this app. The credential you sign here is what " +
        "lets them publish to this group, and what a consumer's node checks.",
    ),
  );
  const member = document.createElement("input");
  member.className = "issue-member-did";
  member.placeholder = "member DID";
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
  const button = text("button", "Issue credential", "button issue-credential") as HTMLButtonElement;
  const status = text("p", "", "issue-status");

  button.onclick = async () => {
    status.textContent = "";
    const chosen = boxes.filter((b) => b.box.checked).map((b) => b.box.value);
    const n = Number.parseInt(days.value, 10);
    if (!member.value.trim()) return void (status.textContent = "Enter the member's DID.");
    if (chosen.length === 0) return void (status.textContent = "Choose at least one category.");
    if (!Number.isInteger(n) || n < 1 || n > 730) {
      return void (status.textContent = "Days must be a whole number from 1 to 730.");
    }
    button.disabled = true;
    try {
      await call("credential.issue", {
        member_did: member.value.trim(),
        categories: chosen,
        expires_at_secs: Math.floor(Date.now() / 1000) + n * 86400,
      });
      status.textContent = "Issued.";
      member.value = "";
      await reload();
    } catch (err) {
      status.textContent = `Not issued: ${errText(err)}`;
    }
    button.disabled = false;
  };
  form.append(
    field("Member DID", member),
    text("div", "Covers:", "issue-categories-label"),
    ...boxes.map((b) => b.label),
    field("Valid for (days)", days),
    button,
    status,
  );
  return form;
}

function buildSuspendForm(reload: () => Promise<void>): HTMLElement {
  const form = document.createElement("div");
  form.className = "suspend-form";
  form.appendChild(text("h4", "Suspend a member"));
  const member = document.createElement("input");
  member.className = "suspend-member-did";
  member.placeholder = "member DID";
  const listing = document.createElement("input");
  listing.className = "suspend-listing-id";
  listing.placeholder = "listing id (leave empty to suspend the whole membership)";
  const rule = document.createElement("input");
  rule.className = "suspend-rule";
  rule.placeholder = "which of your rules";
  const reason = document.createElement("input");
  reason.className = "suspend-reason";
  reason.placeholder = "reason";
  const days = document.createElement("input");
  days.className = "suspend-days";
  days.placeholder = "days (empty = until you lift it)";
  const button = text("button", "Suspend", "button suspend-member") as HTMLButtonElement;
  const status = text("p", "", "suspend-status");

  button.onclick = async () => {
    status.textContent = "";
    if (!member.value.trim() || !rule.value.trim()) {
      status.textContent = "A member DID and the rule are required.";
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
        status.textContent = "Days must be a whole number of at least 1.";
        return;
      }
      params.until_secs = Math.floor(Date.now() / 1000) + n * 86400;
    }
    button.disabled = true;
    try {
      await call("member.suspend", params);
      status.textContent = "Suspended.";
      await reload();
    } catch (err) {
      status.textContent = `Not suspended: ${errText(err)}`;
    }
    button.disabled = false;
  };
  form.append(
    field("Member DID", member),
    field("One listing only", listing),
    field("Rule", rule),
    field("Reason", reason),
    field("For how long", days),
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
  wrap.appendChild(text("h4", "Credentials you have issued"));
  const revoked = new Set(revocations.map((r) => r.about));
  const newestPerMember = new Map<string, IssuedRow>();
  for (const c of credentials) {
    const seen = newestPerMember.get(c.member_did);
    if (!seen || c.issued_at_secs >= seen.issued_at_secs) newestPerMember.set(c.member_did, c);
  }
  if (credentials.length === 0) wrap.appendChild(text("p", "None issued yet.", "credential-empty"));
  for (const row of credentials.slice().sort((a, b) => b.issued_at_secs - a.issued_at_secs)) {
    const view = credentialView(row);
    const isCurrent = newestPerMember.get(row.member_did)?.record_id === row.record_id;
    const status = revoked.has(row.record_id) ? "revoked" : isCurrent ? "current" : "replaced";
    const line = document.createElement("div");
    line.className = "credential-row";
    line.dataset.status = status;
    line.dataset.member = row.member_did;
    line.appendChild(text("span", row.member_did, "credential-member"));
    line.appendChild(text("span", `covers ${view.categories.join(", ")}`, "credential-covers"));
    line.appendChild(
      text("span", `expires ${dateWords(view.expiresAtSecs)}`, "credential-expires"),
    );
    line.appendChild(text("span", status, "credential-status"));
    if (status === "current") line.appendChild(revokeControl(row, reload));
    wrap.appendChild(line);
  }
  return wrap;
}

function revokeControl(row: IssuedRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "revoke-control";
  const reason = document.createElement("input");
  reason.className = "revoke-reason";
  reason.placeholder = "reason";
  const button = text("button", "Revoke", "button revoke-credential") as HTMLButtonElement;
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
      status.textContent = `Not revoked: ${errText(err)}`;
      button.disabled = false;
    }
  };
  wrap.append(reason, button, status);
  return wrap;
}

function buildDecisionList(decisions: IssuedRow[], reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "decision-list";
  wrap.appendChild(text("h4", "Suspensions and lifts"));
  const views = decisions.map(decisionView).sort((a, b) => b.row.issued_at_secs - a.row.issued_at_secs);
  const lifted = new Set(views.filter((v) => v.action === "lift").map((v) => v.row.about));
  if (views.length === 0) wrap.appendChild(text("p", "No decisions yet.", "decision-empty"));
  for (const v of views) {
    const line = document.createElement("div");
    line.className = "decision-row";
    const active = v.action === "suspend" && !lifted.has(v.row.record_id);
    line.dataset.status = v.action === "lift" ? "lift" : active ? "active" : "lifted";
    line.appendChild(text("span", v.row.member_did, "decision-member"));
    line.appendChild(text("span", v.action === "lift" ? "lifted a suspension" : `suspended ${v.scopeWords}`, "decision-what"));
    if (v.action === "suspend") {
      line.appendChild(text("span", `rule: ${v.rule}`, "decision-rule"));
      const until = v.untilSecs ? `until ${dateWords(v.untilSecs)}` : "until lifted";
      line.appendChild(text("span", until, "decision-until"));
    }
    line.appendChild(text("span", dateWords(v.row.issued_at_secs), "decision-date"));
    if (active) line.appendChild(liftControl(v.row, reload));
    wrap.appendChild(line);
  }
  return wrap;
}

function liftControl(row: IssuedRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "lift-control";
  const button = text("button", "Lift", "button lift-suspension") as HTMLButtonElement;
  const status = text("span", "", "lift-status");
  button.onclick = async () => {
    button.disabled = true;
    status.textContent = "";
    try {
      await call("member.lift", { decision_record_id: row.record_id });
      await reload();
    } catch (err) {
      status.textContent = `Not lifted: ${errText(err)}`;
      button.disabled = false;
    }
  };
  wrap.append(button, status);
  return wrap;
}
