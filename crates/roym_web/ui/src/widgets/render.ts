import { t } from "../i18n/i18n";
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
