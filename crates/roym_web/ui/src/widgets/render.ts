import { messageStateWord } from "../chat/message_state";
import { widgetNameId, widgetType } from "../extensions/registry";
import { codeWord, t } from "../i18n/i18n";
import { call } from "../rpc";
import { renderGenericWidget } from "./generic";
import { renderPoll } from "./poll";
import type { StoredState, WidgetRenderOptions, WidgetView } from "./view";

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
): void {
  if (state === "delivered") return;

  const stateSpan = document.createElement("span");
  stateSpan.className = `message-state ${target === "widget" ? "widget-state" : "answer-state"} state-${state}`;
  stateSpan.textContent = messageStateWord(state);
  container.appendChild(stateSpan);

  if (state === "failed") {
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

export function renderWidget(view: WidgetView, opts: WidgetRenderOptions): HTMLElement {
  if (view.refused) {
    const card = document.createElement("div");
    card.className = "widget-card widget-refused";
    card.dataset.widgetId = view.widget_id;

    const p = document.createElement("p");
    p.className = "refused-notice";
    if (view.refused === "widget-too-large") {
      p.textContent = t("widget-too-large");
    } else {
      p.textContent = t("widget-refused", { reason: view.refused });
    }
    card.appendChild(p);

    if (view.direction === "outgoing") {
      stateAndRetry(card, view.state, view.last_error, view.widget_id, "widget", opts.refresh);
    }
    return card;
  }

  if (!view.definition) {
    const card = document.createElement("div");
    card.className = "widget-card widget-refused";
    card.dataset.widgetId = view.widget_id;
    const p = document.createElement("p");
    p.className = "refused-notice";
    p.textContent = t("widget-refused", { reason: "widget-corrupt" });
    card.appendChild(p);
    return card;
  }

  const isPoll =
    view.definition.semantic === "poll" &&
    view.definition.fields.length === 1 &&
    view.definition.fields[0]?.type === "choice" &&
    view.definition.fields[0]?.required === true;

  return isPoll ? renderPoll(view, opts) : renderGenericWidget(view, opts);
}
