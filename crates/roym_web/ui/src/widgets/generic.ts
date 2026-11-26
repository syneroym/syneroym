import { messageLang, messageText } from "../chat/message_text";
import { errText, labelled } from "../dom";
import { widgetFieldLabelId } from "../extensions/registry";
import { formatDateTime, formatDuration, formatNumber } from "../i18n/format";
import { codeWord, t } from "../i18n/i18n";
import { call } from "../rpc";
import { stateAndRetry, widgetHeading } from "./render";
import type { DefinedField, WidgetDefinition, WidgetRenderOptions, WidgetView } from "./view";

function fieldLabel(def: WidgetDefinition, f: DefinedField, isKnown: boolean): string {
  if (isKnown) {
    return codeWord(new Map([[f.name, widgetFieldLabelId(def.extension.id, def.type, f.name)]]), f.name);
  }
  return f.label ?? f.name;
}

export function renderGenericWidget(view: WidgetView, opts: WidgetRenderOptions): HTMLElement {
  const card = document.createElement("div");
  card.className = "widget-card widget-generic";
  card.dataset.widgetId = view.widget_id;

  const def = view.definition;
  if (!def) {
    return card;
  }

  const { isKnown } = widgetHeading(card, view);

  const incoming = view.direction === "incoming";
  const promptNodes = messageText(def.prompt, view.languages ?? [], "p", { translatable: incoming });
  card.append(...promptNodes);

  // Form for answering
  const form = document.createElement("form");
  form.className = "widget-generic-form";
  form.onsubmit = (e) => e.preventDefault();

  for (const f of def.fields) {
    const p = document.createElement("p");
    p.className = `field widget-field widget-field-${f.type}`;
    const label = fieldLabel(def, f, isKnown);

    let inputEl: HTMLElement;
    if (f.type === "long-text") {
      const textarea = document.createElement("textarea");
      textarea.name = f.name;
      if (f.max_len != null) {
        textarea.maxLength = f.max_len;
      }
      inputEl = textarea;
    } else if (f.type === "choice") {
      const select = document.createElement("select");
      select.name = f.name;
      for (const c of f.choices ?? []) {
        const opt = document.createElement("option");
        opt.value = c.code;
        opt.textContent = c.text;
        select.appendChild(opt);
      }
      inputEl = select;
    } else {
      const input = document.createElement("input");
      input.name = f.name;
      if (f.type === "number") {
        input.type = "number";
        if (f.min != null) input.min = String(f.min);
        if (f.max != null) input.max = String(f.max);
      } else if (f.type === "date-time") {
        input.type = "datetime-local";
      } else if (f.type === "duration") {
        input.type = "number";
      } else {
        input.type = "text";
        if (f.max_len != null) input.maxLength = f.max_len;
      }
      inputEl = input;
    }

    labelled(p, label, inputEl);
    form.appendChild(p);
  }

  const submitBtn = document.createElement("button");
  submitBtn.type = "button";
  submitBtn.className = "button widget-answer-submit";
  submitBtn.textContent = t("widget-answer-send");

  const errP = document.createElement("p");
  errP.className = "widget-answer-error";

  submitBtn.onclick = async () => {
    submitBtn.disabled = true;
    errP.textContent = "";
    const data: Record<string, unknown> = {};
    for (const f of def.fields) {
      const item = form.elements.namedItem(f.name) as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null;
      if (!item) continue;
      const val = item.value;
      if (val === "" && !f.required) continue;
      if (f.type === "number") {
        data[f.name] = Number(val);
      } else if (f.type === "date-time") {
        data[f.name] = Math.floor(new Date(val).getTime() / 1000);
      } else if (f.type === "duration") {
        data[f.name] = Number(val) * 60;
      } else {
        data[f.name] = val;
      }
    }
    try {
      await call("widget.answer", {
        conversation: opts.conversation,
        widget_id: view.widget_id,
        data,
        languages: opts.languages ? opts.languages() : [],
      });
      await opts.refresh?.();
    } catch (err) {
      errP.textContent = t("widget-answer-failed", { detail: errText(err) });
      submitBtn.disabled = false;
    }
  };

  form.append(submitBtn, errP);
  card.appendChild(form);

  const ownAnswer = view.answers.find((a) => a.direction === "outgoing");

  if (view.direction === "outgoing") {
    stateAndRetry(card, view.state, view.last_error, view.widget_id, "widget", opts.refresh);
  }

  if (ownAnswer) {
    stateAndRetry(card, ownAnswer.state, ownAnswer.last_error, ownAnswer.message_id, "answer", opts.refresh);
  }

  // Answers section
  if (view.answers.length > 0) {
    const answersSec = document.createElement("div");
    answersSec.className = "widget-answers-section";

    const answersTitle = document.createElement("h4");
    answersTitle.className = "widget-answers-heading";
    answersTitle.textContent = t("widget-answers-heading");
    answersSec.appendChild(answersTitle);

    const list = document.createElement("ul");
    list.className = "widget-answers-list";

    const nonTextFields = def.fields.filter((f) => f.type !== "text" && f.type !== "long-text");
    const textFields = def.fields.filter((f) => f.type === "text" || f.type === "long-text");

    for (const ans of view.answers) {
      const li = document.createElement("li");
      li.className = "widget-answer-item";

      const authorName = opts.nameOf ? opts.nameOf(ans.author) : ans.author;
      const authorSpan = document.createElement("strong");
      authorSpan.className = "answer-author";
      authorSpan.textContent = authorName;
      li.appendChild(authorSpan);

      for (const f of nonTextFields) {
        if (ans.data[f.name] !== undefined) {
          const val = ans.data[f.name];
          const p = document.createElement("p");
          p.className = `field-view-line field-view-${f.type}`;
          const lbl = fieldLabel(def, f, isKnown);

          let valNode: Node;
          if (f.type === "choice") {
            const choice = f.choices?.find((c) => c.code === val);
            const span = document.createElement("span");
            span.textContent = choice ? choice.text : String(val);
            span.setAttribute("lang", messageLang(view.languages ?? []));
            valNode = span;
          } else if (f.type === "number") {
            valNode = document.createTextNode(typeof val === "number" ? formatNumber(val) : String(val));
          } else if (f.type === "date-time") {
            valNode = document.createTextNode(typeof val === "number" ? formatDateTime(val) : String(val));
          } else if (f.type === "duration") {
            valNode = document.createTextNode(typeof val === "number" ? formatDuration(val) : String(val));
          } else {
            valNode = document.createTextNode(String(val));
          }

          labelled(p, lbl, valNode);
          li.appendChild(p);
        }
      }

      for (const tf of textFields) {
        if (ans.data[tf.name] !== undefined) {
          const val = String(ans.data[tf.name]);
          const p = document.createElement("p");
          p.className = `answer-text-line answer-${tf.type}`;
          const lbl = fieldLabel(def, tf, isKnown);
          const labelSpan = document.createElement("span");
          labelSpan.className = "answer-field-label";
          labelSpan.textContent = `${lbl}: `;
          p.appendChild(labelSpan);
          const translatable = ans.direction === "incoming";
          p.append(...messageText(val, ans.languages ?? [], "span", { translatable }));
          li.appendChild(p);
        }
      }

      list.appendChild(li);
    }

    answersSec.appendChild(list);
    card.appendChild(answersSec);
  }

  return card;
}
