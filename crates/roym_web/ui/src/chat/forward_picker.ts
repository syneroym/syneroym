import { errText, text } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";

interface ConversationRow {
  id: string;
  kind?: "direct" | "group";
  peer_address?: string;
  peer_person_did?: string;
  name?: string | null;
}

/**
 * A list of the person's chats. Picking one runs `onPick`; the dialog
 * closes when it succeeds and stays open with the reason when it does not.
 */
export async function openForwardPicker(
  anchor: HTMLElement,
  onPick: (conversationId: string) => Promise<void>,
): Promise<void> {
  anchor.querySelector(".forward-picker")?.remove();

  const dialog = document.createElement("div");
  dialog.className = "forward-picker";
  dialog.appendChild(text("h4", t("forward-picker-title"), "forward-picker-title"));
  const list = document.createElement("div");
  list.className = "forward-picker-list";
  const err = text("p", "", "forward-picker-error error-text");
  const close = text("button", t("common-cancel"), "button forward-picker-close") as HTMLButtonElement;
  close.onclick = () => dialog.remove();
  dialog.append(list, err, close);

  const buttons: HTMLButtonElement[] = [];
  const setDisabled = (disabled: boolean) => {
    for (const b of buttons) b.disabled = disabled;
  };

  try {
    const res = await call<{ conversations: ConversationRow[] }>("conversation.list", {});
    const rows = res.conversations ?? [];
    if (rows.length === 0) {
      list.appendChild(text("p", t("forward-picker-empty"), "forward-picker-empty"));
    }
    for (const row of rows) {
      const label = row.name || row.peer_person_did || row.peer_address || row.id;
      const button = text("button", label, "button forward-picker-row") as HTMLButtonElement;
      button.onclick = async () => {
        err.textContent = "";
        setDisabled(true);
        try {
          await onPick(row.id);
          dialog.remove();
        } catch (e) {
          err.textContent = t("forward-refused", { detail: errText(e) });
          setDisabled(false);
        }
      };
      buttons.push(button);
      list.appendChild(button);
    }
  } catch (e) {
    list.appendChild(text("p", t("forward-picker-load-failed", { detail: errText(e) }), "forward-picker-load-failed"));
  }

  anchor.appendChild(dialog);
}
