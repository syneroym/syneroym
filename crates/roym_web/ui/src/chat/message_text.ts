import { t } from "../i18n/i18n";
import { langAttr } from "../i18n/text_lang";

/// The `lang` value for a message: langAttr of the first tag, or "".
export function messageLang(languages: readonly string[]): string {
  if (languages.length === 0) {
    return "";
  }
  return langAttr(languages[0]);
}

/// The text of a chat message: a `tag` element (default "span") of class "message-body" with the
/// body, and `lang` from the first tag (langAttr), or "" when there is no
/// tag. When `languages` is empty, a second <span class="message-language-note">
/// with "Language not stated". Returns the nodes to append.
export function messageText(
  body: string,
  languages: readonly string[],
  tag = "span",
): Node[] {
  const el = document.createElement(tag);
  el.className = "message-body";
  el.textContent = body;
  el.setAttribute("lang", messageLang(languages));

  if (languages.length === 0) {
    const note = document.createElement("span");
    note.className = "message-language-note";
    note.textContent = t("chat-text-not-stated");
    return [el, note];
  }
  return [el];
}
