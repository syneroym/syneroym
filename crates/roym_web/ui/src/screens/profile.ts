import { errText, field } from "../dom";
import { currentLanguage, t } from "../i18n/i18n";
import { languageList } from "../i18n/language_list";
import { textLangSelect } from "../i18n/text_lang";
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

  let langSelect = textLangSelect("profile-text-lang", currentLanguage());
  form.appendChild(field(t("profile-text-language"), langSelect));

  const langHeading = document.createElement("h3");
  langHeading.textContent = t("profile-languages-heading");
  form.appendChild(langHeading);

  let storedAbout: string | undefined;
  let storedTranslations: unknown[] = [];

  const langList = languageList(
    "profile",
    8,
    [currentLanguage()],
    {
      add: t("profile-languages-add"),
      moveUp: t("profile-languages-move-up"),
      remove: t("profile-languages-remove"),
    },
    t("profile-languages-heading"),
  );
  form.appendChild(langList.root);

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
        languages: langList.value(),
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
          const freshSelect = textLangSelect("profile-text-lang", env.payload.lang);
          langSelect.replaceWith(freshSelect);
          langSelect = freshSelect;
        }
        if (Array.isArray(env.payload.languages)) {
          langList.set(env.payload.languages);
        }
      }
    }
  } catch {
    // initial load optional
  }

  container.appendChild(box);
}
