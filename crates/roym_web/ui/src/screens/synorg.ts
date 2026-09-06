import { call, RpcError } from "../rpc";

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

function errText(err: unknown): string {
  if (err instanceof RpcError) return err.message;
  return err instanceof Error ? err.message : String(err);
}

function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

function field(labelText: string, control: HTMLElement): HTMLElement {
  const label = document.createElement("label");
  label.className = "field";
  label.appendChild(text("span", labelText));
  label.appendChild(control);
  return label;
}

export async function renderSynOrg(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "synorg-screen";
  box.appendChild(text("h2", "Your SynOrg"));
  box.appendChild(
    text(
      "p",
      "A SynOrg is a directory you run: providers publish their own listings " +
        "to it, and anyone can search it. Creating settings is what turns this " +
        "installation into one.",
      "synorg-intro",
    ),
  );

  let current: SynOrgSettings | null = null;
  try {
    current = await call<SynOrgSettings | null>("directory.settings");
  } catch (err) {
    box.appendChild(text("p", `Could not load settings: ${errText(err)}`));
  }

  box.appendChild(
    text(
      "p",
      current ? `This installation runs the SynOrg "${current.name}".` : "This installation runs no SynOrg yet.",
      "synorg-status",
    ),
  );

  box.appendChild(buildSettingsForm(current));

  if (current) {
    box.appendChild(buildLimitEditor(current.publication_limits));
    box.appendChild(await buildRoster());
    box.appendChild(await buildPublications());
  }

  container.appendChild(box);
}

function buildSettingsForm(current: SynOrgSettings | null): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "synorg-settings-form";
  wrap.appendChild(text("h3", current ? "Edit settings" : "Create your SynOrg"));

  const name = document.createElement("input");
  name.className = "synorg-name";
  name.placeholder = "name";
  name.value = current?.name ?? "";
  const rules = document.createElement("textarea");
  rules.className = "synorg-rules";
  rules.placeholder = "the rules a person reads before deciding to trust this group";
  rules.value = current?.rules ?? "";
  const categories = document.createElement("input");
  categories.className = "synorg-categories";
  categories.placeholder = "categories, comma separated";
  categories.value = (current?.categories ?? []).join(", ");
  const support = document.createElement("input");
  support.className = "synorg-support";
  support.placeholder = "support contact";
  support.value = current?.support_contact ?? "";
  const dispute = document.createElement("textarea");
  dispute.className = "synorg-dispute";
  dispute.placeholder = "how a dispute is handled";
  dispute.value = current?.dispute_path ?? "";
  const retentionDays = document.createElement("input");
  retentionDays.className = "synorg-retention-days";
  retentionDays.placeholder = "how many days a publication is kept";
  retentionDays.value = String(Math.round((current?.retention_secs ?? 30 * 24 * 3600) / 86400));

  const saveBtn = text("button", "Save settings", "button save-synorg") as HTMLButtonElement;
  const status = text("p", "", "synorg-save-status");

  saveBtn.onclick = async () => {
    status.textContent = "";
    const days = Number.parseInt(retentionDays.value, 10);
    if (!Number.isInteger(days) || days < 1) {
      status.textContent = "Retention must be a whole number of days.";
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
      status.textContent = "Saved.";
      const statusLine = document.querySelector(".synorg-status");
      if (statusLine) statusLine.textContent = `This installation runs the SynOrg "${name.value.trim()}".`;
    } catch (err) {
      status.textContent = `Save failed: ${errText(err)}`;
    }
    saveBtn.disabled = false;
  };

  wrap.append(
    field("Name", name),
    field("Rules", rules),
    field("Categories", categories),
    field("Support contact", support),
    field("Dispute path", dispute),
    field("Retention (days)", retentionDays),
    saveBtn,
    status,
  );
  return wrap;
}

/// The publication limit, editable here rather than a hidden default: the
/// likely case is a provider with a large catalogue joining and being
/// refused partway through their first publish, and the group's owner is
/// the person who should set the limit.
function buildLimitEditor(limits: { window_secs: number; max_per_window: number }): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "synorg-limit-editor";
  wrap.appendChild(text("h3", "Publication limit"));
  wrap.appendChild(
    text(
      "p",
      "How many listings one provider may publish here inside one window. " +
        "Taking a listing down never counts against this.",
    ),
  );
  const windowInput = document.createElement("input");
  windowInput.className = "limit-window";
  windowInput.placeholder = "window (seconds)";
  windowInput.value = String(limits.window_secs);
  const maxInput = document.createElement("input");
  maxInput.className = "limit-max";
  maxInput.placeholder = "max publications per window";
  maxInput.value = String(limits.max_per_window);
  const saveBtn = text("button", "Save limit", "button save-limit") as HTMLButtonElement;
  const status = text("p", "", "limit-status");

  saveBtn.onclick = async () => {
    status.textContent = "";
    const w = Number.parseInt(windowInput.value, 10);
    const m = Number.parseInt(maxInput.value, 10);
    if (!Number.isInteger(w) || !Number.isInteger(m)) {
      status.textContent = "Both values must be whole numbers.";
      return;
    }
    saveBtn.disabled = true;
    try {
      await call("directory.set-limits", { window_secs: w, max_per_window: m });
      status.textContent = "Saved.";
    } catch (err) {
      status.textContent = `Save failed: ${errText(err)}`;
    }
    saveBtn.disabled = false;
  };

  wrap.append(windowInput, maxInput, saveBtn, status);
  return wrap;
}

async function buildRoster(): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "synorg-roster";
  wrap.appendChild(text("h3", "Members"));
  wrap.appendChild(
    text("p", "The roster is local. It is never served over the wire.", "roster-note"),
  );

  const list = document.createElement("div");
  list.className = "roster-list";
  const didInput = document.createElement("input");
  didInput.className = "roster-did";
  didInput.placeholder = "member DID";
  const noteInput = document.createElement("input");
  noteInput.className = "roster-note-input";
  noteInput.placeholder = "note (optional)";
  const addBtn = text("button", "Add member", "button add-member") as HTMLButtonElement;
  const status = text("p", "", "roster-status");

  const reload = async () => {
    list.replaceChildren();
    try {
      const res = await call<{ members: Array<{ did: string; note: string }> }>("member.list");
      for (const m of res.members) {
        const line = document.createElement("div");
        line.className = "roster-row";
        line.appendChild(text("span", m.did, "member-did"));
        if (m.note) line.appendChild(text("span", m.note, "member-note"));
        const rm = text("button", "Remove", "button remove-member") as HTMLButtonElement;
        rm.onclick = async () => {
          await call("member.remove", { did: m.did });
          await reload();
        };
        line.appendChild(rm);
        list.appendChild(line);
      }
    } catch (err) {
      list.appendChild(text("p", `Could not load members: ${errText(err)}`));
    }
  };

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
      status.textContent = `Add failed: ${errText(err)}`;
    }
  };

  await reload();
  wrap.append(list, didInput, noteInput, addBtn, status);
  return wrap;
}

async function buildPublications(): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "synorg-publications";
  wrap.appendChild(text("h3", "Published here"));
  try {
    const res = await call<{ publications: Array<{ listing_id: string; issuer: string; published_by: string }> }>(
      "directory.publications",
    );
    if (res.publications.length === 0) {
      wrap.appendChild(text("p", "Nothing published here yet."));
    }
    for (const p of res.publications) {
      const line = document.createElement("div");
      line.className = "publication-row";
      line.appendChild(text("span", p.listing_id, "publication-listing"));
      line.appendChild(text("span", `by ${p.issuer}`, "publication-issuer"));
      const rm = text("button", "Remove", "button unpublish") as HTMLButtonElement;
      rm.onclick = async () => {
        await call("directory.unpublish", { listing_id: p.listing_id });
        rm.disabled = true;
        line.appendChild(text("span", "removed from future search", "publication-removed"));
      };
      line.appendChild(rm);
      wrap.appendChild(line);
    }
  } catch (err) {
    wrap.appendChild(text("p", `Could not load publications: ${errText(err)}`));
  }
  return wrap;
}
