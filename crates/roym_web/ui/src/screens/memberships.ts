import {
  checkedWords,
  issuerChanged,
  membershipWords,
  pinnedIssuerWords,
  type CheckStandingReply,
  type MembershipVerdict,
} from "../directory/membership";
import { errText, text } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";

/// One check this node has made, as `directory.memberships` returns it. The
/// verdict is worked out from the stored evidence at the moment of the
/// call, never read from a stored verdict.
interface HeldMembership {
  source: string;
  member_did: string;
  issuer_did: string | null;
  as_of_secs: number;
  last_error?: string | null;
  verdict: MembershipVerdict;
}

async function sourceLabels(): Promise<Map<string, string>> {
  const labels = new Map<string, string>();
  try {
    const res = await call<{ sources: Array<{ did: string; label: string }> }>("directory.sources");
    for (const s of res.sources ?? []) labels.set(s.did, s.label);
  } catch {
    /* a DID is a fine label */
  }
  return labels;
}

/// The memberships this node has checked. A group's decision reaches a
/// copy held here only when the person checks again -- both notices stay
/// on screen, above the list, whatever it holds.
export async function renderMemberships(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "memberships-screen";
  box.appendChild(text("h2", t("memberships-title")));
  box.appendChild(text("p", t("memberships-intro"), "memberships-intro"));
  box.appendChild(text("p", t("membership-no-instant-removal"), "no-instant-removal-notice"));
  box.appendChild(text("p", t("membership-withheld-revocation"), "withheld-revocation-notice"));

  const list = document.createElement("div");
  list.className = "memberships-list";
  box.appendChild(list);
  container.appendChild(box);

  let rows: HeldMembership[] = [];
  try {
    rows = (await call<{ memberships: HeldMembership[] }>("directory.memberships")).memberships ?? [];
  } catch (err) {
    list.appendChild(text("p", t("memberships-load-failed", { detail: errText(err) })));
    return;
  }
  if (rows.length === 0) {
    list.appendChild(text("p", t("memberships-empty"), "memberships-empty"));
    return;
  }
  const labels = await sourceLabels();
  for (const row of rows) list.appendChild(buildRow(row, labels.get(row.source) || row.source));
}

function buildRow(row: HeldMembership, label: string): HTMLElement {
  const line = document.createElement("div");
  line.className = "membership-row";
  line.dataset.state = row.verdict.state;
  line.appendChild(text("div", t("membership-row-source", { label }), "membership-source"));
  line.appendChild(text("div", t("membership-row-member", { did: row.member_did }), "membership-member"));
  const words = text("div", membershipWords(row.verdict, label), "membership-words");
  const checked = text("div", checkedWords(row.as_of_secs), "membership-checked");
  line.append(words, checked);
  line.appendChild(text("div", pinnedIssuerWords(row.issuer_did), "membership-issuer"));
  if (row.last_error) {
    line.appendChild(
      text("div", t("membership-last-error", { error: row.last_error }), "membership-last-error"),
    );
  }

  const again = text("button", t("membership-check-again"), "button check-again") as HTMLButtonElement;
  const status = text("div", "", "membership-check-status");
  again.onclick = async () => {
    again.disabled = true;
    status.textContent = "";
    try {
      const res = await call<CheckStandingReply>("directory.check-standing", {
        source: row.source,
        member_did: row.member_did,
      });
      if (issuerChanged(res)) {
        // The reply was not used, so the row keeps showing the held copy
        // (the one a reload shows) and only the status line says why.
        status.textContent = t("membership-issuer-changed-check");
      } else {
        line.dataset.state = res.verdict.state;
        words.textContent = membershipWords(res.verdict, label);
        checked.textContent = res.refreshed
          ? t("membership-checked", { age: t("age-moments") })
          : checkedWords(res.as_of_secs ?? 0);
        status.textContent = res.refreshed
          ? t("membership-checked-now")
          : t("membership-unreachable");
      }
    } catch (err) {
      status.textContent = t("membership-check-failed", { detail: errText(err) });
    }
    again.disabled = false;
  };
  line.append(again, status);
  return line;
}
