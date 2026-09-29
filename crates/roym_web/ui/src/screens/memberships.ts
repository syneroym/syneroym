import {
  checkedWords,
  ISSUER_CHANGED_CHECK_WORDS,
  issuerChanged,
  membershipWords,
  NO_INSTANT_REMOVAL_NOTICE,
  pinnedIssuerWords,
  WITHHELD_REVOCATION_NOTICE,
  type CheckStandingReply,
  type MembershipVerdict,
} from "../directory/membership";
import { errText, text } from "../dom";
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
  box.appendChild(text("h2", "Memberships"));
  box.appendChild(
    text(
      "p",
      "Each line is a provider's membership in a group, as this node worked it out from the " +
        "group's own signed statements. It is the copy you hold, not a live answer.",
      "memberships-intro",
    ),
  );
  box.appendChild(text("p", NO_INSTANT_REMOVAL_NOTICE, "no-instant-removal-notice"));
  box.appendChild(text("p", WITHHELD_REVOCATION_NOTICE, "withheld-revocation-notice"));

  const list = document.createElement("div");
  list.className = "memberships-list";
  box.appendChild(list);
  container.appendChild(box);

  let rows: HeldMembership[] = [];
  try {
    rows = (await call<{ memberships: HeldMembership[] }>("directory.memberships")).memberships ?? [];
  } catch (err) {
    list.appendChild(text("p", `Could not load memberships: ${errText(err)}`));
    return;
  }
  if (rows.length === 0) {
    list.appendChild(
      text(
        "p",
        "No memberships checked yet. One appears here after a search returns a result from a " +
          "group the provider belongs to.",
        "memberships-empty",
      ),
    );
    return;
  }
  const labels = await sourceLabels();
  for (const row of rows) list.appendChild(buildRow(row, labels.get(row.source) || row.source));
}

function buildRow(row: HeldMembership, label: string): HTMLElement {
  const line = document.createElement("div");
  line.className = "membership-row";
  line.dataset.state = row.verdict.state;
  line.appendChild(text("div", `Group directory: ${label}`, "membership-source"));
  line.appendChild(text("div", `Provider: ${row.member_did}`, "membership-member"));
  const words = text("div", membershipWords(row.verdict, label), "membership-words");
  const checked = text("div", checkedWords(row.as_of_secs), "membership-checked");
  line.append(words, checked);
  line.appendChild(text("div", pinnedIssuerWords(row.issuer_did), "membership-issuer"));
  if (row.last_error) {
    line.appendChild(text("div", `Last check failed: ${row.last_error}`, "membership-last-error"));
  }

  const again = text("button", "Check again", "button check-again") as HTMLButtonElement;
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
        status.textContent = ISSUER_CHANGED_CHECK_WORDS;
      } else {
        line.dataset.state = res.verdict.state;
        words.textContent = membershipWords(res.verdict, label);
        checked.textContent = res.refreshed ? "checked moments ago" : checkedWords(res.as_of_secs ?? 0);
        status.textContent = res.refreshed
          ? "Checked just now."
          : "Could not reach this directory; this is the copy you already held.";
      }
    } catch (err) {
      status.textContent = `Could not check: ${errText(err)}`;
    }
    again.disabled = false;
  };
  line.append(again, status);
  return line;
}
