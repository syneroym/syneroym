import { call } from "../rpc";

export function upsertParams(
  did: string,
  addr: string,
  envText: string,
): Record<string, unknown> {
  const params: Record<string, unknown> = {
    person_did: did.trim(),
  };
  const trimmedAddr = addr.trim();
  if (trimmedAddr) {
    params["conversation_address"] = trimmedAddr;
  }
  const trimmedEnv = envText.trim();
  if (trimmedEnv) {
    params["profile_envelope"] = trimmedEnv;
  }
  return params;
}

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
      const trimmedEnv = envInput.value.trim();
      if (trimmedEnv) {
        try {
          JSON.parse(trimmedEnv);
        } catch {
          errorP.textContent = "Profile envelope is not valid JSON";
          return;
        }
      }
      const params = upsertParams(didInput.value, addrInput.value, envInput.value);
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
