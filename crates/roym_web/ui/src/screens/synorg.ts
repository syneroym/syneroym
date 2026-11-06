import { errText, field, input, refiller, text } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import { buildMembersPanel } from "./synorg_members";

interface SynOrgSettings {
  name: string;
  rules: string;
  area: unknown[];
  categories: string[];
  support_contact: string;
  dispute_path: string;
  retention_secs: number;
  publication_limits: { window_secs: number; max_per_window: number };
}

export async function renderSynOrg(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "synorg-screen";
  box.appendChild(text("h2", t("synorg-title")));
  box.appendChild(text("p", t("synorg-intro"), "synorg-intro"));

  let current: SynOrgSettings | null = null;
  try {
    current = await call<SynOrgSettings | null>("directory.settings");
  } catch (err) {
    box.appendChild(text("p", t("synorg-settings-load-failed", { detail: errText(err) })));
  }

  box.appendChild(
    text(
      "p",
      current ? t("synorg-status-running", { name: current.name }) : t("synorg-status-none"),
      "synorg-status",
    ),
  );

  box.appendChild(buildSettingsForm(current));

  if (current) {
    box.appendChild(buildLimitEditor(current.publication_limits));
    box.appendChild(await buildRoster());
    box.appendChild(await buildMembersPanel(current.categories));
    box.appendChild(await buildPublications());
  }

  container.appendChild(box);
}

function buildSettingsForm(current: SynOrgSettings | null): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "synorg-settings-form";
  wrap.appendChild(text("h3", current ? t("synorg-settings-edit") : t("synorg-settings-create")));

  const name = document.createElement("input");
  name.className = "synorg-name";
  name.placeholder = t("synorg-name-placeholder");
  name.value = current?.name ?? "";
  const rules = document.createElement("textarea");
  rules.className = "synorg-rules";
  rules.placeholder = t("synorg-rules-placeholder");
  rules.value = current?.rules ?? "";
  const categories = document.createElement("input");
  categories.className = "synorg-categories";
  categories.placeholder = t("synorg-categories-placeholder");
  categories.value = (current?.categories ?? []).join(", ");
  const support = document.createElement("input");
  support.className = "synorg-support";
  support.placeholder = t("synorg-support-placeholder");
  support.value = current?.support_contact ?? "";
  const dispute = document.createElement("textarea");
  dispute.className = "synorg-dispute";
  dispute.placeholder = t("synorg-dispute-placeholder");
  dispute.value = current?.dispute_path ?? "";
  const retentionDays = document.createElement("input");
  retentionDays.className = "synorg-retention-days";
  retentionDays.placeholder = t("synorg-retention-placeholder");
  retentionDays.value = String(Math.round((current?.retention_secs ?? 30 * 24 * 3600) / 86400));

  const saveBtn = text("button", t("synorg-save-button"), "button save-synorg") as HTMLButtonElement;
  const status = text("p", "", "synorg-save-status");

  saveBtn.onclick = async () => {
    status.textContent = "";
    const days = Number.parseInt(retentionDays.value, 10);
    if (!Number.isInteger(days) || days < 1) {
      status.textContent = t("synorg-retention-invalid");
      return;
    }
    const limits = current?.publication_limits ?? { window_secs: 24 * 3600, max_per_window: 20 };
    saveBtn.disabled = true;
    try {
      await call("directory.set-settings", {
        name: name.value.trim(),
        rules: rules.value,
        area: current?.area ?? [],
        categories: categories.value
          .split(",")
          .map((c) => c.trim())
          .filter(Boolean),
        support_contact: support.value.trim(),
        dispute_path: dispute.value,
        retention_secs: days * 86400,
        publication_limits: limits,
      });
      status.textContent = t("common-saved");
      const statusLine = document.querySelector(".synorg-status");
      if (statusLine) {
        statusLine.textContent = t("synorg-status-running", { name: name.value.trim() });
      }
    } catch (err) {
      status.textContent = t("common-save-failed", { detail: errText(err) });
    }
    saveBtn.disabled = false;
  };

  wrap.append(
    field(t("synorg-label-name"), name),
    field(t("synorg-label-rules"), rules),
    field(t("synorg-label-categories"), categories),
    field(t("synorg-label-support"), support),
    field(t("synorg-label-dispute"), dispute),
    field(t("synorg-label-retention"), retentionDays),
    saveBtn,
    status,
  );
  return wrap;
}

/// The publication limit, editable here rather than a hidden default: the
/// likely case is a listing owner with a large catalogue joining and being
/// refused partway through their first publish, and the group's owner is
/// the person who should set the limit.
function buildLimitEditor(limits: { window_secs: number; max_per_window: number }): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "synorg-limit-editor";
  wrap.appendChild(text("h3", t("synorg-limit-heading")));
  wrap.appendChild(text("p", t("synorg-limit-intro")));
  const windowInput = input(t("synorg-limit-window-placeholder"));
  windowInput.className = "limit-window";
  windowInput.value = String(limits.window_secs);
  const maxInput = input(t("synorg-limit-max-placeholder"));
  maxInput.className = "limit-max";
  maxInput.value = String(limits.max_per_window);
  const saveBtn = text("button", t("synorg-limit-save"), "button save-limit") as HTMLButtonElement;
  const status = text("p", "", "limit-status");

  saveBtn.onclick = async () => {
    status.textContent = "";
    const w = Number.parseInt(windowInput.value, 10);
    const m = Number.parseInt(maxInput.value, 10);
    if (!Number.isInteger(w) || !Number.isInteger(m)) {
      status.textContent = t("synorg-limit-whole");
      return;
    }
    saveBtn.disabled = true;
    try {
      await call("directory.set-limits", { window_secs: w, max_per_window: m });
      status.textContent = t("common-saved");
    } catch (err) {
      status.textContent = t("common-save-failed", { detail: errText(err) });
    }
    saveBtn.disabled = false;
  };

  wrap.append(windowInput, maxInput, saveBtn, status);
  return wrap;
}

async function buildRoster(): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "synorg-roster";
  wrap.appendChild(text("h3", t("synorg-roster-heading")));
  wrap.appendChild(text("p", t("synorg-roster-note"), "roster-note"));

  const list = document.createElement("div");
  list.className = "roster-list";
  const didInput = input(t("synorg-member-did-placeholder"));
  didInput.className = "roster-did";
  const noteInput = input(t("synorg-roster-note-placeholder"));
  noteInput.className = "roster-note-input";
  const addBtn = text("button", t("synorg-roster-add"), "button add-member") as HTMLButtonElement;
  const status = text("p", "", "roster-status");

  const reload = refiller(list, async (into) => {
    try {
      const res = await call<{ members: Array<{ did: string; note: string }> }>("member.list");
      for (const m of res.members) {
        const line = document.createElement("div");
        line.className = "roster-row";
        line.appendChild(text("span", m.did, "member-did"));
        if (m.note) line.appendChild(text("span", m.note, "member-note"));
        const rm = text("button", t("common-remove"), "button remove-member") as HTMLButtonElement;
        const refusal = text("span", "", "member-remove-error");
        rm.onclick = async () => {
          refusal.textContent = "";
          try {
            await call("member.remove", { did: m.did });
          } catch (err) {
            // Refused while the member's credential still stands: say why on
            // this row, and leave the roster as it is.
            refusal.textContent = t("synorg-roster-not-removed", { detail: errText(err) });
            return;
          }
          await reload();
        };
        line.append(rm, refusal);
        into.appendChild(line);
      }
    } catch (err) {
      into.appendChild(text("p", t("synorg-roster-load-failed", { detail: errText(err) })));
    }
  });

  addBtn.onclick = async () => {
    status.textContent = "";
    const did = didInput.value.trim();
    if (!did) return;
    try {
      await call("member.add", { did, note: noteInput.value.trim() });
      didInput.value = "";
      noteInput.value = "";
      await reload();
    } catch (err) {
      status.textContent = t("synorg-roster-add-failed", { detail: errText(err) });
    }
  };

  await reload();
  wrap.append(list, didInput, noteInput, addBtn, status);
  return wrap;
}

async function buildPublications(): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "synorg-publications";
  wrap.appendChild(text("h3", t("synorg-published-heading")));
  try {
    const res = await call<{ publications: Array<{ listing_id: string; issuer: string; published_by: string }> }>(
      "directory.publications",
    );
    if (res.publications.length === 0) {
      wrap.appendChild(text("p", t("synorg-published-empty")));
    }
    for (const p of res.publications) {
      const line = document.createElement("div");
      line.className = "publication-row";
      line.appendChild(text("span", p.listing_id, "publication-listing"));
      line.appendChild(text("span", t("synorg-published-by", { issuer: p.issuer }), "publication-issuer"));
      const rm = text("button", t("common-remove"), "button unpublish") as HTMLButtonElement;
      rm.onclick = async () => {
        await call("directory.unpublish", { listing_id: p.listing_id });
        rm.disabled = true;
        line.appendChild(text("span", t("synorg-unpublished"), "publication-removed"));
      };
      line.appendChild(rm);
      wrap.appendChild(line);
    }
  } catch (err) {
    wrap.appendChild(text("p", t("synorg-publications-load-failed", { detail: errText(err) })));
  }
  return wrap;
}
