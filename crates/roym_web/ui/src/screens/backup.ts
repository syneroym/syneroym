import { call } from "../rpc";

/// The five app-data bundles Roym can export today. Each service owns its
/// own bundle; a single signed bundle that composes them comes later.
const BUNDLES: Array<{ label: string; exportMethod: string; importMethod: string; file: string }> = [
  {
    label: "Profile, contacts, and safety records",
    exportMethod: "profile.export",
    importMethod: "profile.import",
    file: "roym-profile-bundle.json",
  },
  {
    label: "Conversations and messages",
    exportMethod: "conversation.export",
    importMethod: "conversation.import",
    file: "roym-conversation-bundle.json",
  },
  {
    label: "Listings and availability",
    exportMethod: "catalog.export",
    importMethod: "catalog.import",
    file: "roym-catalog-bundle.json",
  },
  {
    label: "Requests, quotes, and agreements",
    exportMethod: "transaction.export",
    importMethod: "transaction.import",
    file: "roym-transaction-bundle.json",
  },
  {
    label: "This installation's SynOrg directory, if it runs one",
    exportMethod: "directory.export",
    importMethod: "directory.import",
    file: "roym-directory-bundle.json",
  },
];

function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

export async function renderBackup(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "backup-screen";

  box.appendChild(text("h2", "App Data Backup & Restore"));
  box.appendChild(
    text(
      "p",
      "This exports and imports application data. It never exports or touches your identity secret key.",
    ),
  );
  box.appendChild(
    text(
      "p",
      "The five bundles below are exported separately today; a single signed bundle that combines them comes later.",
      "backup-separate-note",
    ),
  );

  for (const b of BUNDLES) {
    box.appendChild(bundleRow(b));
  }

  container.appendChild(box);
}

function bundleRow(b: (typeof BUNDLES)[number]): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "bundle-row";
  wrap.dataset.export = b.exportMethod;
  wrap.appendChild(text("h3", b.label));

  const status = text("p", "", "bundle-status");

  const exportBtn = text("button", "Export", "button bundle-export") as HTMLButtonElement;
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
      status.textContent = `Export failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  const importLabel = document.createElement("label");
  importLabel.className = "button bundle-import-label";
  importLabel.textContent = "Import";
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = "application/json,.json";
  fileInput.style.display = "none";
  fileInput.onchange = async () => {
    const f = fileInput.files?.[0];
    if (!f) return;
    status.textContent = "Reading file...";
    try {
      const textContent = await f.text();
      const parsed = JSON.parse(textContent);
      status.textContent = "Importing...";
      const res = await call<{ imported_records?: number; imported?: number }>(b.importMethod, { bundle: parsed });
      const count = res?.imported_records ?? res?.imported ?? "bundle";
      status.textContent = `Imported: ${count} record(s).`;
    } catch (err) {
      status.textContent = `Import failed: ${err instanceof Error ? err.message : String(err)}`;
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
