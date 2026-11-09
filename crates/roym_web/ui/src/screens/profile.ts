import { errText, field } from "../dom";
import { currentLanguage, t } from "../i18n/i18n";
import { RECORD_LANGUAGES, languageName, textLangSelect } from "../i18n/text_lang";
import { call } from "../rpc";

export async function renderProfile(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "profile-screen";

  const h2 = document.createElement("h2");
  h2.textContent = t("profile-title");
  box.appendChild(h2);

  const form = document.createElement("form");
  const nameLabel = document.createElement("label");
  nameLabel.append(t("profile-display-name"), " ");
  const nameInput = document.createElement("input");
  nameInput.type = "text";
  nameLabel.appendChild(nameInput);
  form.appendChild(nameLabel);

  const addrLabel = document.createElement("label");
  addrLabel.append(" ", t("profile-conversation-address"), " ");
  const addrInput = document.createElement("input");
  addrInput.type = "text";
  addrLabel.appendChild(addrInput);
  form.appendChild(addrLabel);

  const langSelect = textLangSelect("profile-text-lang", currentLanguage());
  form.appendChild(field(t("profile-text-language"), langSelect));

  const langHeading = document.createElement("h3");
  langHeading.textContent = t("profile-languages-heading");
  form.appendChild(langHeading);

  let languages: string[] = [currentLanguage()];
  let storedAbout: string | undefined;
  let storedTranslations: unknown[] = [];

  const listContainer = document.createElement("ul");
  listContainer.className = "profile-languages-list";
  form.appendChild(listContainer);

  const addSelect = document.createElement("select");
  addSelect.className = "profile-languages-add";
  form.appendChild(addSelect);

  function renderLanguageRows() {
    listContainer.replaceChildren();
    for (let idx = 0; idx < languages.length; idx++) {
      const tag = languages[idx];
      const li = document.createElement("li");
      li.className = "profile-language-row";

      const nameSpan = document.createElement("span");
      nameSpan.className = "profile-language-name";
      nameSpan.textContent = languageName(tag);
      li.appendChild(nameSpan);

      if (idx > 0) {
        const upBtn = document.createElement("button");
        upBtn.type = "button";
        upBtn.className = "button profile-language-move-up";
        upBtn.textContent = t("profile-languages-move-up");
        upBtn.onclick = () => {
          const temp = languages[idx - 1];
          languages[idx - 1] = languages[idx];
          languages[idx] = temp;
          renderLanguageRows();
        };
        li.appendChild(upBtn);
      }

      const rmBtn = document.createElement("button");
      rmBtn.type = "button";
      rmBtn.className = "button profile-language-remove";
      rmBtn.textContent = t("profile-languages-remove");
      rmBtn.onclick = () => {
        languages.splice(idx, 1);
        renderLanguageRows();
      };
      li.appendChild(rmBtn);

      listContainer.appendChild(li);
    }

    addSelect.replaceChildren();
    const defaultOpt = document.createElement("option");
    defaultOpt.value = "";
    defaultOpt.textContent = t("profile-languages-add");
    addSelect.appendChild(defaultOpt);

    for (const tag of RECORD_LANGUAGES) {
      if (!languages.includes(tag)) {
        const opt = document.createElement("option");
        opt.value = tag;
        opt.textContent = languageName(tag);
        addSelect.appendChild(opt);
      }
    }
    addSelect.disabled = languages.length >= 8;
  }

  addSelect.onchange = () => {
    if (addSelect.value) {
      languages.push(addSelect.value);
      renderLanguageRows();
      addSelect.value = "";
    }
  };

  renderLanguageRows();

  const saveBtn = document.createElement("button");
  saveBtn.type = "submit";
  saveBtn.className = "button";
  saveBtn.textContent = t("profile-save");
  form.appendChild(saveBtn);

  const status = document.createElement("p");
  form.appendChild(status);

  form.onsubmit = async (e) => {
    e.preventDefault();
    status.textContent = t("profile-saving");
    try {
      const res = await call<{ record_id: string; envelope: string }>("profile.set", {
        display_name: nameInput.value,
        conversation_address: addrInput.value || undefined,
        lang: langSelect.value,
        languages,
        about: storedAbout,
        translations: storedTranslations.length > 0 ? storedTranslations : undefined,
      });
      const env = typeof res.envelope === "string" ? JSON.parse(res.envelope) : res.envelope;
      status.textContent = t("profile-saved", {
        record: res.record_id || "",
        issuer: env.issuer || "",
      });
    } catch (err) {
      status.textContent = t("profile-error", { detail: errText(err) });
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
        storedAbout = env.payload.about;
        storedTranslations = env.payload.translations || [];
        if (env.payload.lang) {
          langSelect.value = env.payload.lang;
        }
        if (Array.isArray(env.payload.languages) && env.payload.languages.length > 0) {
          languages = [...env.payload.languages];
        }
        renderLanguageRows();
      }
    }
  } catch {
    // initial load optional
  }

  container.appendChild(box);
}
