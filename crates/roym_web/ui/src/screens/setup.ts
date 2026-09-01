import { call } from "../rpc";

export async function renderSetup(container: HTMLElement, onDone: () => void) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "setup-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "Signing Certificate Required";
  box.appendChild(h2);

  const desc = document.createElement("p");
  desc.textContent =
    "This installation has no active record-signing certificate enrolled. " +
    "To enrol, run `roymctl roym enrol-signing --master <identity> --registry-url <registry>` from your terminal. " +
    "The browser cannot do this because it holds no master key.";
  box.appendChild(desc);

  const checkBtn = document.createElement("button");
  checkBtn.className = "button";
  checkBtn.textContent = "Check again";
  checkBtn.onclick = async () => {
    try {
      const res = await call<{ certificate: { state: string } }>("profile.signing-status");
      if (res.certificate?.state === "installed") {
        onDone();
      }
    } catch {
      // stay on setup screen
    }
  };
  box.appendChild(checkBtn);

  container.appendChild(box);
}
