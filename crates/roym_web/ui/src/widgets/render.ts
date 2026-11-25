import { t } from "../i18n/i18n";
import { call } from "../rpc";
import { messageStateWord } from "../screens/messages";
import { renderGenericWidget } from "./generic";
import { renderPoll } from "./poll";
import type { WidgetRenderOptions, WidgetView } from "./view";

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

    if (view.direction === "outgoing" && view.state !== "delivered") {
      const stateSpan = document.createElement("span");
      stateSpan.className = `message-state widget-state state-${view.state}`;
      stateSpan.textContent = messageStateWord(view.state);
      card.appendChild(stateSpan);

      if (view.state === "failed") {
        if (view.last_error) {
          const errSpan = document.createElement("span");
          errSpan.className = "message-error widget-error";
          errSpan.textContent = view.last_error;
          card.appendChild(errSpan);
        }
        const retryBtn = document.createElement("button");
        retryBtn.type = "button";
        retryBtn.className = "button retry-message retry-widget";
        retryBtn.textContent = t("messages-retry-button");
        retryBtn.onclick = async () => {
          retryBtn.disabled = true;
          try {
            await call("conversation.retry", { message_id: view.widget_id });
            await opts.refresh?.();
          } catch {
            retryBtn.disabled = false;
          }
        };
        card.appendChild(retryBtn);
      }
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
