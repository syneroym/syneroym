import { availableLanguages, currentLanguage, saveLanguage, t } from "./i18n";
import { languageName } from "./text_lang";

/// A list of the languages the Hub has a string catalog for. A choice is
/// saved and the page reloads, because each screen draws its text once.
/// When the browser does not keep the choice, a reload would bring back
/// the old language and look like nothing happened, so the Hub says so
/// instead.
export function languagePicker(reload: () => void = () => window.location.reload()): HTMLSelectElement {
  const select = document.createElement("select");
  select.className = "language-picker";
  select.setAttribute("aria-label", t("language-picker-label"));
  for (const tag of availableLanguages()) {
    const option = document.createElement("option");
    option.value = tag;
    option.textContent = languageName(tag, tag);
    select.appendChild(option);
  }
  select.value = currentLanguage();

  select.onchange = () => {
    if (saveLanguage(select.value)) {
      reload();
      return;
    }
    select.value = currentLanguage();
    if (select.parentNode && !select.parentNode.querySelector(".language-note")) {
      const note = document.createElement("span");
      note.className = "language-note";
      note.textContent = t("language-not-saved");
      select.after(note);
    }
  };
  return select;
}
