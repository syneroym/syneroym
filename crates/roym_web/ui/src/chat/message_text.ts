import { t } from "../i18n/i18n";
import { langAttr, languageName } from "../i18n/text_lang";
import {
  type Engine,
  keptTranslation,
  machineNote,
  targetFor,
  translateTexts,
  translationFailure,
} from "../translation/translation";

export interface MessageTextOptions {
  /// True for a text message from someone else. Only then, and only when the
  /// node can translate it into a language the reader reads, is a translate
  /// button drawn. The person's own messages and cards never get one.
  translatable?: boolean;
}

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
/// with "Language not stated". A translate button follows the body when the
/// message is `translatable` and the node can translate it; the body itself
/// never changes. Returns the nodes to append.
export function messageText(
  body: string,
  languages: readonly string[],
  tag = "span",
  opts: MessageTextOptions = {},
): Node[] {
  const el = document.createElement(tag);
  el.className = "message-body";
  el.textContent = body;
  el.setAttribute("lang", messageLang(languages));

  const nodes: Node[] = [el];
  if (languages.length === 0) {
    const note = document.createElement("span");
    note.className = "message-language-note";
    note.textContent = t("chat-text-not-stated");
    nodes.push(note);
  }
  const target = opts.translatable ? targetFor(languages) : null;
  if (target !== null) nodes.push(translateButton(body, languages, target));
  return nodes;
}

/// A button that translates `body` once. The translation is drawn after the
/// button, in its own span with its own `lang`, with a note naming the
/// engine. When the node refuses, the reason is drawn and the body stays.
/// A translation this page already made of the same text is drawn at once,
/// with no button, so a redraw of the thread does not lose it or ask the
/// engine again.
function translateButton(body: string, languages: readonly string[], target: string): HTMLElement {
  const box = document.createElement("span");
  box.className = "message-translate-box";
  const sourceLang = languages[0] ?? null;
  const earlier = keptTranslation([body], sourceLang, target);
  if (earlier) {
    showTranslation(box, earlier.texts[0] ?? "", earlier.engine, target);
    return box;
  }
  const button = document.createElement("button");
  button.type = "button";
  button.className = "button message-translate";
  button.textContent = t("translation-translate-button", { language: languageName(target) });
  button.onclick = async () => {
    button.disabled = true;
    box.querySelector(".message-translation-error")?.remove();
    try {
      const out = await translateTexts([body], sourceLang, target);
      button.remove();
      showTranslation(box, out.texts[0] ?? "", out.engine, target);
    } catch (err) {
      const failure = document.createElement("span");
      failure.className = "message-translation-error";
      failure.textContent = translationFailure(err);
      box.appendChild(failure);
      button.disabled = false;
    }
  };
  box.appendChild(button);
  return box;
}

function showTranslation(box: HTMLElement, text: string, engine: Engine, target: string): void {
  const translated = document.createElement("span");
  translated.className = "message-translation";
  translated.setAttribute("lang", langAttr(target));
  translated.textContent = text;
  const note = document.createElement("span");
  note.className = "message-translation-note";
  note.textContent = machineNote(target, engine);
  box.append(translated, note);
}
