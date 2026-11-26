import { messageStateWord } from "../chat/message_state";
import { widgetNameId, widgetType } from "../extensions/registry";
import { codeWord, t } from "../i18n/i18n";
import { call } from "../rpc";
import type { StoredState, WidgetView } from "./view";

export function widgetHeading(card: HTMLElement, view: WidgetView): { isKnown: boolean } {
  const def = view.definition;
  const isKnown = view.checked || (def ? widgetType(def.extension, def.type) !== undefined : false);
  const title = document.createElement("h3");
  title.className = "widget-heading";
  title.textContent =
    isKnown && def
      ? codeWord(new Map([[def.type, widgetNameId(def.extension.id, def.type)]]), def.type)
      : t("widget-unknown-heading", { type: def?.type ?? "" });
  card.appendChild(title);

  if (!view.checked) {
    const notChecked = document.createElement("p");
    notChecked.className = "widget-not-checked field-view-not-checked";
    notChecked.textContent = t("ext-not-checked");
    card.appendChild(notChecked);
  }

  return { isKnown };
}

export function stateAndRetry(
  container: HTMLElement,
  state: StoredState,
  lastError: string | null | undefined,
  messageId: string,
  target: "widget" | "answer",
  refresh?: () => Promise<void> | void,
  restored: boolean = false,
): void {
  if (state === "delivered") return;

  const stateSpan = document.createElement("span");
  stateSpan.className = `message-state ${target === "widget" ? "widget-state" : "answer-state"} state-${state}`;
  stateSpan.textContent = messageStateWord(state);
  container.appendChild(stateSpan);

  if (state === "failed" && !restored) {
    if (lastError) {
      const errSpan = document.createElement("span");
      errSpan.className = `message-error ${target === "widget" ? "widget-error" : "answer-error"}`;
      errSpan.textContent = lastError;
      container.appendChild(errSpan);
    }
    const retryBtn = document.createElement("button");
    retryBtn.type = "button";
    retryBtn.className = `button retry-message ${target === "widget" ? "retry-widget" : "retry-answer"}`;
    retryBtn.textContent = t("messages-retry-button");
    retryBtn.onclick = async () => {
      retryBtn.disabled = true;
      try {
        await call("conversation.retry", { message_id: messageId });
        await refresh?.();
      } catch {
        retryBtn.disabled = false;
      }
    };
    container.appendChild(retryBtn);
  }
}
