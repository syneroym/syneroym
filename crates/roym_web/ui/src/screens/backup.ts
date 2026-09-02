import { call } from "../rpc";

export async function renderBackup(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "backup-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "App Data Backup & Restore";
  box.appendChild(h2);

  const note = document.createElement("p");
  note.textContent =
    "This exports and imports application data (profile, contacts, safety records). " +
    "It never exports or touches your identity secret key.";
  box.appendChild(note);

  const exportBtn = document.createElement("button");
  exportBtn.className = "button";
  exportBtn.textContent = "Export App Data Bundle";
  exportBtn.onclick = async () => {
    try {
      const bundle = await call("profile.export");
      const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "roym-app-data-bundle.json";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      alert(`Export failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };
  box.appendChild(exportBtn);

  const importLabel = document.createElement("label");
  importLabel.className = "button file-button";
  importLabel.textContent = "Import App Data Bundle";
  const importInput = document.createElement("input");
  importInput.type = "file";
  importInput.accept = ".json";
  importInput.style.display = "none";
  importInput.onchange = async () => {
    const file = importInput.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      const bundle = JSON.parse(text);
      await call("profile.import", { bundle });
      alert("App data restored successfully.");
    } catch (err) {
      alert(`Import failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };
  importLabel.appendChild(importInput);
  box.appendChild(importLabel);

  container.appendChild(box);
}
