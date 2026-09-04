import { call } from "../rpc";

export async function renderSafety(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "safety-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "Safety & Blocklist";
  box.appendChild(h2);

  const blockDesc = document.createElement("p");
  box.appendChild(blockDesc);

  try {
    const policy = await call<{ statement?: string }>("profile.policy");
    if (policy?.statement) {
      blockDesc.textContent = policy.statement;
    } else {
      blockDesc.textContent = "Could not load this node's safety policy.";
    }
  } catch {
    blockDesc.textContent = "Could not load this node's safety policy.";
  }

  const blockInput = document.createElement("input");
  blockInput.className = "block-input";
  blockInput.placeholder = "Person DID or Address to block";
  box.appendChild(blockInput);

  const blockBtn = document.createElement("button");
  blockBtn.className = "button";
  blockBtn.textContent = "Block";

  const blockError = document.createElement("p");
  blockError.className = "block-error";
  box.appendChild(blockBtn);
  box.appendChild(blockError);

  blockBtn.onclick = async () => {
    blockError.textContent = "";
    const val = blockInput.value.trim();
    if (!val) return;
    try {
      if (val.startsWith("did:")) {
        await call("block.add", { person_did: val });
      } else {
        await call("block.add", { address: val });
      }
      blockInput.value = "";
      await loadBlocks();
    } catch (err) {
      blockError.textContent = `Block failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  const blockList = document.createElement("div");
  box.appendChild(blockList);

  const loadBlocks = async () => {
    blockList.replaceChildren();
    try {
      const rows = await call<Array<{ person_did?: string; address?: string }>>("block.list");
      for (const r of rows) {
        const item = document.createElement("div");
        item.textContent = `Blocked: ${r.person_did || r.address || ""}`;
        blockList.appendChild(item);
      }
    } catch {
      blockList.textContent = "Could not load block list.";
    }
  };
  await loadBlocks();

  box.appendChild(buildReportForm());
  box.appendChild(await buildContactLimitEditor());

  container.appendChild(box);
}

function textNode(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

const REPORT_CATEGORIES = [
  "impersonation",
  "fraud",
  "harassment",
  "unsafe-service",
  "illegal-content",
];

/// File and list safety reports. The `[PRD-SAF]` backlog row deferred this
/// surface from the identity/profile slice to here.
function buildReportForm(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "report-form";
  wrap.appendChild(textNode("h3", "Report a person, listing, or message"));

  const kind = document.createElement("select");
  for (const k of ["person", "listing", "message"]) {
    const o = document.createElement("option");
    o.value = k;
    o.textContent = k;
    kind.appendChild(o);
  }
  const subjectId = document.createElement("input");
  subjectId.placeholder = "Subject DID or id";
  const category = document.createElement("select");
  for (const c of REPORT_CATEGORIES) {
    const o = document.createElement("option");
    o.value = c;
    o.textContent = c;
    category.appendChild(o);
  }
  const details = document.createElement("textarea");
  details.placeholder = "What happened (optional)";

  const fileBtn = textNode("button", "File report", "button file-report") as HTMLButtonElement;
  const status = textNode("p", "", "report-status");
  const list = document.createElement("div");
  list.className = "report-list";

  const reload = async () => {
    list.replaceChildren();
    try {
      const rows = await call<
        Array<{ report_id: string; subject_kind: string; subject_id: string; category: string; status: string }>
      >("report.list");
      for (const r of rows) {
        const line = document.createElement("div");
        line.className = "report-row";
        line.appendChild(
          textNode("span", `${r.category} - ${r.subject_kind} ${r.subject_id} [${r.status}]`),
        );
        if (r.status !== "withdrawn") {
          const wd = textNode("button", "Withdraw", "button withdraw-report") as HTMLButtonElement;
          wd.onclick = async () => {
            await call("report.withdraw", { report_id: r.report_id });
            await reload();
          };
          line.appendChild(wd);
        }
        list.appendChild(line);
      }
    } catch {
      list.textContent = "Could not load reports.";
    }
  };

  fileBtn.onclick = async () => {
    status.textContent = "";
    const sid = subjectId.value.trim();
    if (!sid) {
      status.textContent = "A subject is required.";
      return;
    }
    try {
      const res = await call<{ report_id: string; status: string }>("report.create", {
        subject_kind: kind.value,
        subject_id: sid,
        category: category.value,
        details: details.value.trim() || undefined,
      });
      status.textContent = `Recorded ${res.report_id} (${res.status})`;
      subjectId.value = "";
      details.value = "";
      await reload();
    } catch (err) {
      status.textContent = `Report failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  wrap.append(kind, subjectId, category, details, fileBtn, status, list);
  void reload();
  return wrap;
}

/// The first-contact rate limit, mirroring the catalog's publication-limit
/// editor exactly.
async function buildContactLimitEditor(): Promise<HTMLElement> {
  const wrap = document.createElement("div");
  wrap.className = "contact-limit-editor";
  wrap.appendChild(textNode("h3", "First-contact rate limit"));
  wrap.appendChild(
    textNode(
      "p",
      "How many strangers may start a conversation with you inside one window. " +
        "An ongoing conversation is never limited.",
    ),
  );

  const windowInput = document.createElement("input");
  windowInput.placeholder = "window (seconds)";
  const maxInput = document.createElement("input");
  maxInput.placeholder = "max new contacts per window";
  const saveBtn = textNode("button", "Save limit", "button save-contact-limit") as HTMLButtonElement;
  const status = textNode("p", "", "contact-limit-status");

  try {
    const limits = await call<{ window_secs: number; max_per_window: number }>("contacts.limits");
    windowInput.value = String(limits.window_secs);
    maxInput.value = String(limits.max_per_window);
  } catch {
    status.textContent = "Could not load the current limit.";
  }

  saveBtn.onclick = async () => {
    status.textContent = "";
    const w = Number.parseInt(windowInput.value, 10);
    const m = Number.parseInt(maxInput.value, 10);
    if (!Number.isInteger(w) || !Number.isInteger(m)) {
      status.textContent = "Both values must be whole numbers.";
      return;
    }
    try {
      await call("contacts.set-limits", { window_secs: w, max_per_window: m });
      status.textContent = "Saved.";
    } catch (err) {
      status.textContent = `Save failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  wrap.append(windowInput, maxInput, saveBtn, status);
  return wrap;
}
