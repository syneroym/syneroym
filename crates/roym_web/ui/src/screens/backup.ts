import { errText, text } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";

/// The five app-data bundles Roym can export today. Each service owns its
/// own bundle; a single signed bundle that composes them comes later.
const BUNDLES: Array<{ label: () => string; exportMethod: string; importMethod: string; file: string }> = [
  {
    label: () => t("backup-bundle-profile"),
    exportMethod: "profile.export",
    importMethod: "profile.import",
    file: "roym-profile-bundle.json",
  },
  {
    label: () => t("backup-bundle-conversation"),
    exportMethod: "conversation.export",
    importMethod: "conversation.import",
    file: "roym-conversation-bundle.json",
  },
  {
    label: () => t("backup-bundle-catalog"),
    exportMethod: "catalog.export",
    importMethod: "catalog.import",
    file: "roym-catalog-bundle.json",
  },
  {
    label: () => t("backup-bundle-transaction"),
    exportMethod: "transaction.export",
    importMethod: "transaction.import",
    file: "roym-transaction-bundle.json",
  },
  {
    label: () => t("backup-bundle-directory"),
    exportMethod: "directory.export",
    importMethod: "directory.import",
    file: "roym-directory-bundle.json",
  },
];

/// What the services answer to an import: a map of counts (conversation,
/// catalog, directory), a flag (transaction), or other fields (profile).
interface ImportReply {
  imported_records?: unknown;
  imported?: unknown;
}

/// The number of records an import reply reports, or `undefined` when it
/// reports none. A map of counts is added up; a flag or an unknown shape is
/// not a number, so the Hub says it imported the bundle without a count.
export function importedCount(reply: ImportReply | null | undefined): number | undefined {
  const reported = reply?.imported_records ?? reply?.imported;
  if (typeof reported === "number") return reported;
  if (reported === null || typeof reported !== "object") return undefined;
  const counts = Object.values(reported).filter((v): v is number => typeof v === "number");
  return counts.length > 0 ? counts.reduce((sum, v) => sum + v, 0) : undefined;
}

export async function renderBackup(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "backup-screen";

  box.appendChild(text("h2", t("backup-title")));
  box.appendChild(text("p", t("backup-intro")));
  box.appendChild(text("p", t("backup-separate-note"), "backup-separate-note"));

  for (const b of BUNDLES) {
    box.appendChild(bundleRow(b));
  }

  container.appendChild(box);
}

function bundleRow(b: (typeof BUNDLES)[number]): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "bundle-row";
  wrap.dataset.export = b.exportMethod;
  wrap.appendChild(text("h3", b.label()));

  const status = text("p", "", "bundle-status");

  const exportBtn = text("button", t("backup-export"), "button bundle-export") as HTMLButtonElement;
  exportBtn.onclick = async () => {
    status.textContent = "";
    try {
      const bundle = await call(b.exportMethod);
      const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = b.file;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      status.textContent = t("backup-export-failed", { detail: errText(err) });
    }
  };

  const importLabel = document.createElement("label");
  importLabel.className = "button bundle-import-label";
  importLabel.textContent = t("backup-import");
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = "application/json,.json";
  fileInput.style.display = "none";
  fileInput.onchange = async () => {
    const f = fileInput.files?.[0];
    if (!f) return;
    status.textContent = t("backup-reading");
    try {
      const textContent = await f.text();
      const parsed = JSON.parse(textContent);
      status.textContent = t("backup-importing");
      const res = await call<ImportReply | null>(b.importMethod, { bundle: parsed });
      const count = importedCount(res);
      status.textContent =
        count === undefined ? t("backup-imported-unknown") : t("backup-imported", { count });
    } catch (err) {
      status.textContent = t("backup-import-failed", { detail: errText(err) });
    }
    fileInput.value = "";
  };
  importLabel.appendChild(fileInput);

  const actions = document.createElement("div");
  actions.className = "bundle-actions";
  actions.append(exportBtn, importLabel);

  wrap.append(actions, status);
  return wrap;
}
