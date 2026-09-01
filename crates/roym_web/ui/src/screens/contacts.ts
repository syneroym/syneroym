import { call } from "../rpc";

export async function renderContacts(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "contacts-screen";

  const h2 = document.createElement("h2");
  h2.textContent = "Contacts";
  box.appendChild(h2);

  const listDiv = document.createElement("div");
  box.appendChild(listDiv);

  const reload = async () => {
    listDiv.replaceChildren();
    try {
      const contacts = await call<Array<{ person_did: string; display_name?: string; favourite?: boolean }>>("contacts.list");
      for (const c of contacts) {
        const item = document.createElement("div");
        item.textContent = `${c.display_name || c.person_did} ${c.favourite ? "★" : ""}`;
        listDiv.appendChild(item);
      }
    } catch {
      listDiv.textContent = "Could not load contacts.";
    }
  };
  await reload();

  const addHeading = document.createElement("h3");
  addHeading.textContent = "Add / Import Contact";
  box.appendChild(addHeading);

  const didInput = document.createElement("input");
  didInput.placeholder = "Person DID";
  box.appendChild(didInput);

  const addrInput = document.createElement("input");
  addrInput.placeholder = "Conversation Address (required if no profile envelope)";
  box.appendChild(addrInput);

  const envInput = document.createElement("textarea");
  envInput.placeholder = "Paste Profile Envelope JSON (optional)";
  box.appendChild(envInput);

  const errorP = document.createElement("p");
  errorP.className = "upsert-error";
  box.appendChild(errorP);

  const addBtn = document.createElement("button");
  addBtn.className = "button";
  addBtn.textContent = "Upsert Contact";
  addBtn.onclick = async () => {
    errorP.textContent = "";
    try {
      let envObj = undefined;
      if (envInput.value.trim()) {
        envObj = JSON.parse(envInput.value.trim());
      }
      const params: Record<string, unknown> = {
        person_did: didInput.value.trim(),
        profile_envelope: envObj,
      };
      const addr = addrInput.value.trim();
      if (addr) params["conversation_address"] = addr;
      await call("contacts.upsert", params);
      didInput.value = "";
      addrInput.value = "";
      envInput.value = "";
      await reload();
    } catch (err) {
      errorP.textContent = `Upsert failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  };
  box.appendChild(addBtn);

  container.appendChild(box);
}
