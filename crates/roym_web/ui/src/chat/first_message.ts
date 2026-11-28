import { text } from "../dom";
import { t } from "../i18n/i18n";
import { loadForwardSettings, saveForwardSettings, type ForwardMode } from "./forward_mode";

/**
 * The one-time question before the person's first message.
 *
 * Returns `null` when the question was already answered, so the composer
 * keeps whatever mode the person has chosen for this message. When it asks,
 * it returns the answer and saves it as the default. Skip and closing the
 * question keep the shipped default (`not-forwardable`).
 */
export async function askOnce(anchor: HTMLElement): Promise<ForwardMode | null> {
  const settings = await loadForwardSettings();
  if (settings.asked) return null;

  return new Promise<ForwardMode>((resolve) => {
    const dialog = document.createElement("div");
    dialog.className = "forward-mode-dialog";
    dialog.appendChild(text("h4", t("forward-first-title"), "forward-first-title"));
    dialog.appendChild(
      text("p", t("forward-first-signed-explain"), "forward-first-signed-explain"),
    );
    dialog.appendChild(
      text("p", t("forward-first-not-forwardable-explain"), "forward-first-not-forwardable-explain"),
    );
    dialog.appendChild(text("p", t("forward-first-to-whom-note"), "forward-first-to-whom-note"));

    const finish = async (mode: ForwardMode, asked: boolean, saveDefault: boolean) => {
      dialog.remove();
      try {
        await saveForwardSettings({
          ...(saveDefault ? { default_mode: mode } : {}),
          ...(asked ? { asked: true } : {}),
        });
      } catch {
        // The send still goes with the person's answer; the next send asks
        // again, which is better than blocking the message.
      }
      resolve(mode);
    };

    const signed = text("button", t("forward-mode-signed"), "button forward-first-signed") as HTMLButtonElement;
    const notForwardable = text(
      "button",
      t("forward-mode-not-forwardable"),
      "button forward-first-not-forwardable",
    ) as HTMLButtonElement;
    const skip = text("button", t("forward-first-skip"), "button forward-first-skip") as HTMLButtonElement;
    const close = text("button", "×", "button forward-first-close") as HTMLButtonElement;
    close.title = t("forward-first-skip");

    signed.onclick = () => void finish("signed", true, true);
    notForwardable.onclick = () => void finish("not-forwardable", true, true);
    skip.onclick = () => void finish("not-forwardable", true, false);
    close.onclick = () => void finish("not-forwardable", true, false);

    const buttons = document.createElement("div");
    buttons.className = "forward-first-buttons";
    buttons.append(signed, notForwardable, skip);
    dialog.append(buttons, close);
    anchor.appendChild(dialog);
  });
}
