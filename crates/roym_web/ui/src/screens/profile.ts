import { call } from "../rpc";

export async function renderProfile(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "profile-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "Profile";
  box.appendChild(h2);

  const form = document.createElement("form");
  const nameLabel = document.createElement("label");
  nameLabel.textContent = "Display Name: ";
  const nameInput = document.createElement("input");
  nameInput.type = "text";
  nameLabel.appendChild(nameInput);
  form.appendChild(nameLabel);

  const addrLabel = document.createElement("label");
  addrLabel.textContent = " Conversation Address: ";
  const addrInput = document.createElement("input");
  addrInput.type = "text";
  addrLabel.appendChild(addrInput);
  form.appendChild(addrLabel);

  const saveBtn = document.createElement("button");
  saveBtn.type = "submit";
  saveBtn.className = "button";
  saveBtn.textContent = "Save Profile";
  form.appendChild(saveBtn);

  const status = document.createElement("p");
  form.appendChild(status);

  form.onsubmit = async (e) => {
    e.preventDefault();
    status.textContent = "Saving...";
    try {
      const res = await call<{ record_id: string; envelope: string }>("profile.set", {
        display_name: nameInput.value,
        conversation_address: addrInput.value || undefined,
      });
      const env = typeof res.envelope === "string" ? JSON.parse(res.envelope) : res.envelope;
      status.textContent = `Saved record ${res.record_id || ""} (issuer: ${env.issuer || ""})`;
    } catch (err) {
      status.textContent = `Error: ${err instanceof Error ? err.message : String(err)}`;
    }
  };

  box.appendChild(form);

  try {
    const data = await call<{ envelope: string | null }>("profile.get");
    if (data && data.envelope) {
      const env = typeof data.envelope === "string" ? JSON.parse(data.envelope) : data.envelope;
      if (env.payload) {
        nameInput.value = env.payload.display_name || "";
        addrInput.value = env.payload.conversation_address || "";
      }
    }
  } catch {
    // initial load optional
  }

  container.appendChild(box);
}
