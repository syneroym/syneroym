import { codeOptions, errText, input, refiller, select, text } from "../dom";
import { codeWord, messageId, t } from "../i18n/i18n";
import { call } from "../rpc";

export async function renderSafety(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "safety-screen";

  const h2 = document.createElement("h2");
  h2.textContent = t("safety-title");
  box.appendChild(h2);

  const blockDesc = document.createElement("p");
  box.appendChild(blockDesc);

  try {
    const policy = await call<{ statement?: string }>("profile.policy");
    if (policy?.statement) {
      blockDesc.textContent = policy.statement;
    } else {
      blockDesc.textContent = t("safety-policy-unavailable");
    }
  } catch {
    blockDesc.textContent = t("safety-policy-unavailable");
  }

  const blockInput = document.createElement("input");
  blockInput.className = "block-input";
  blockInput.placeholder = t("safety-block-placeholder");
  box.appendChild(blockInput);

  const blockBtn = document.createElement("button");
  blockBtn.className = "button";
  blockBtn.textContent = t("safety-block-button");

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
      blockError.textContent = t("safety-block-failed", { detail: errText(err) });
    }
  };

  const blockList = document.createElement("div");
  box.appendChild(blockList);

  const loadBlocks = refiller(blockList, async (into) => {
    try {
      const rows = await call<Array<{ person_did?: string; address?: string }>>("block.list");
      for (const r of rows) {
        const item = document.createElement("div");
        item.textContent = t("safety-blocked-line", { who: r.person_did || r.address || "" });
        into.appendChild(item);
      }
    } catch {
      into.textContent = t("safety-block-list-failed");
    }
  });
  await loadBlocks();

  box.appendChild(buildReportForm());
  box.appendChild(await buildContactLimitEditor());

  container.appendChild(box);
}

/// Message id for each code shown as a word. A `Map`, so a code from
/// another node can never hit an inherited property; a code not in the
/// table is shown as it came.
const REPORT_CATEGORIES = new Map<string, string>([
  ["impersonation", messageId("safety-category-impersonation")],
  ["fraud", messageId("safety-category-fraud")],
  ["harassment", messageId("safety-category-harassment")],
  ["unsafe-service", messageId("safety-category-unsafe-service")],
  ["illegal-content", messageId("safety-category-illegal-content")],
]);
const REPORT_KINDS = new Map<string, string>([
  ["person", messageId("safety-kind-person")],
  ["listing", messageId("safety-kind-listing")],
  ["message", messageId("safety-kind-message")],
]);
const REPORT_STATUSES = new Map<string, string>([
  ["open", messageId("safety-status-open")],
  ["withdrawn", messageId("safety-status-withdrawn")],
]);

/// File and list safety reports. The `[PRD-SAF]` backlog row deferred this
/// surface from the identity/profile slice to here.
function buildReportForm(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "report-form";
  wrap.appendChild(text("h3", t("safety-report-heading")));

  const kind = select(codeOptions(REPORT_KINDS, [...REPORT_KINDS.keys()]));
  const subjectId = input(t("safety-subject-placeholder"));
  const category = select(codeOptions(REPORT_CATEGORIES, [...REPORT_CATEGORIES.keys()]));
  const details = document.createElement("textarea");
  details.placeholder = t("safety-details-placeholder");

  const fileBtn = text("button", t("safety-file-report"), "button file-report") as HTMLButtonElement;
  const status = text("p", "", "report-status");
  const list = document.createElement("div");
  list.className = "report-list";

  const reload = refiller(list, async (into) => {
    try {
      const rows = await call<
        Array<{ report_id: string; subject_kind: string; subject_id: string; category: string; status: string }>
      >("report.list");
      for (const r of rows) {
        const line = document.createElement("div");
        line.className = "report-row";
        line.appendChild(
          text(
            "span",
            t("safety-report-row", {
              category: codeWord(REPORT_CATEGORIES, r.category),
              kind: codeWord(REPORT_KINDS, r.subject_kind),
              subject: r.subject_id,
              status: codeWord(REPORT_STATUSES, r.status),
            }),
          ),
        );
        if (r.status !== "withdrawn") {
          const wd = text("button", t("safety-withdraw"), "button withdraw-report") as HTMLButtonElement;
          wd.onclick = async () => {
            await call("report.withdraw", { report_id: r.report_id });
            await reload();
          };
          line.appendChild(wd);
        }
        into.appendChild(line);
      }
    } catch {
      into.textContent = t("safety-reports-load-failed");
    }
  });

  fileBtn.onclick = async () => {
    status.textContent = "";
    const sid = subjectId.value.trim();
    if (!sid) {
      status.textContent = t("safety-subject-required");
      return;
    }
    try {
      const res = await call<{ report_id: string; status: string }>("report.create", {
        subject_kind: kind.value,
        subject_id: sid,
        category: category.value,
        details: details.value.trim() || undefined,
      });
      status.textContent = t("safety-recorded", {
        id: res.report_id,
        status: codeWord(REPORT_STATUSES, res.status),
      });
      subjectId.value = "";
      details.value = "";
      await reload();
    } catch (err) {
      status.textContent = t("safety-report-failed", { detail: errText(err) });
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
  wrap.appendChild(text("h3", t("safety-limit-heading")));
  wrap.appendChild(text("p", t("safety-limit-intro")));

  const windowInput = input(t("safety-window-placeholder"));
  const maxInput = input(t("safety-max-placeholder"));
  const saveBtn = text("button", t("safety-save-limit"), "button save-contact-limit") as HTMLButtonElement;
  const status = text("p", "", "contact-limit-status");

  try {
    const limits = await call<{ window_secs: number; max_per_window: number }>("contacts.limits");
    windowInput.value = String(limits.window_secs);
    maxInput.value = String(limits.max_per_window);
  } catch {
    status.textContent = t("safety-limit-load-failed");
  }

  saveBtn.onclick = async () => {
    status.textContent = "";
    const w = Number.parseInt(windowInput.value, 10);
    const m = Number.parseInt(maxInput.value, 10);
    if (!Number.isInteger(w) || !Number.isInteger(m)) {
      status.textContent = t("safety-whole-numbers");
      return;
    }
    try {
      await call("contacts.set-limits", { window_secs: w, max_per_window: m });
      status.textContent = t("common-saved");
    } catch (err) {
      status.textContent = t("safety-save-failed", { detail: errText(err) });
    }
  };

  wrap.append(windowInput, maxInput, saveBtn, status);
  return wrap;
}
