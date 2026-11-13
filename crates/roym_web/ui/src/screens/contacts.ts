import { errText, refiller } from "../dom";
import { formatLocale, t } from "../i18n/i18n";
import { sortByName } from "../names";
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
  h2.textContent = t("contacts-title");
  box.appendChild(h2);

  const searchInput = document.createElement("input");
  searchInput.className = "contacts-search-input";
  searchInput.placeholder = t("contacts-search-placeholder");
  box.appendChild(searchInput);

  const searchBtn = document.createElement("button");
  searchBtn.className = "button contacts-search-button";
  searchBtn.type = "button";
  searchBtn.textContent = t("contacts-search-button");
  box.appendChild(searchBtn);

  const listDiv = document.createElement("div");
  box.appendChild(listDiv);

  const reload = refiller(listDiv, async (into) => {
    try {
      const query = searchInput.value.trim();
      const params = query ? { text: query } : {};
      const contacts = await call<
        Array<{ person_did: string; display_name?: string; favourite?: boolean }>
      >("contacts.list", params);
      if (contacts.length === 0) {
        if (query) {
          const empty = document.createElement("div");
          empty.className = "contacts-search-empty";
          empty.textContent = t("contacts-search-empty");
          into.appendChild(empty);
        }
        return;
      }
      const sorted = sortByName(contacts, (c) => c.display_name ?? "", formatLocale());
      for (const c of sorted) {
        const item = document.createElement("div");
        item.textContent = `${c.display_name || c.person_did} ${c.favourite ? "★" : ""}`;
        into.appendChild(item);
      }
    } catch {
      into.textContent = t("contacts-load-failed");
    }
  });

  searchBtn.onclick = () => {
    void reload();
  };
  searchInput.onkeydown = (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void reload();
    }
  };

  await reload();

  const addHeading = document.createElement("h3");
  addHeading.textContent = t("contacts-add-heading");
  box.appendChild(addHeading);

  const didInput = document.createElement("input");
  didInput.className = "contacts-did-input";
  didInput.placeholder = t("contacts-did-placeholder");
  box.appendChild(didInput);

  const addrInput = document.createElement("input");
  addrInput.className = "contacts-address-input";
  addrInput.placeholder = t("contacts-address-placeholder");
  box.appendChild(addrInput);

  const envInput = document.createElement("textarea");
  envInput.className = "contacts-envelope-input";
  envInput.placeholder = t("contacts-envelope-placeholder");
  box.appendChild(envInput);

  const errorP = document.createElement("p");
  errorP.className = "upsert-error";
  box.appendChild(errorP);

  const addBtn = document.createElement("button");
  addBtn.className = "button contacts-upsert-button";
  addBtn.textContent = t("contacts-upsert-button");
  addBtn.onclick = async () => {
    errorP.textContent = "";
    try {
      const trimmedEnv = envInput.value.trim();
      if (trimmedEnv) {
        try {
          JSON.parse(trimmedEnv);
        } catch {
          errorP.textContent = t("contacts-envelope-invalid");
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
      errorP.textContent = t("contacts-upsert-failed", { detail: errText(err) });
    }
  };
  box.appendChild(addBtn);

  container.appendChild(box);
}
