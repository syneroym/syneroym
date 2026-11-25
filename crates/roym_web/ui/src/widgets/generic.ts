import { fieldView } from "../cards/fields";
import { messageText } from "../chat/message_text";
import { labelled } from "../dom";
import { widgetFieldLabelId, widgetNameId, widgetType } from "../extensions/registry";
import type { DefinedField, WidgetDefinition, WidgetRenderOptions, WidgetView } from "./view";
import { codeWord, errText, t } from "../i18n/i18n";
import { call } from "../rpc";
import { messageStateWord } from "../screens/messages";
import type { FieldDef, SchemaDef } from "../extensions/registry";

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

  const isKnown = view.checked || widgetType(def.extension, def.type) !== undefined;
  const title = document.createElement("h3");
  title.className = "widget-heading";
  title.textContent = isKnown
    ? codeWord(new Map([[def.type, widgetNameId(def.extension.id, def.type)]]), def.type)
    : t("widget-unknown-heading", { type: def.type });
  card.appendChild(title);

  if (!view.checked) {
    const notChecked = document.createElement("p");
    notChecked.className = "widget-not-checked field-view-not-checked";
    notChecked.textContent = t("ext-not-checked");
    card.appendChild(notChecked);
  }

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

  if (view.direction === "outgoing" && view.state !== "delivered") {
    const widgetStateSpan = document.createElement("span");
    widgetStateSpan.className = `message-state widget-state state-${view.state}`;
    widgetStateSpan.textContent = messageStateWord(view.state);
    card.appendChild(widgetStateSpan);

    if (view.state === "failed") {
      if (view.last_error) {
        const widgetErr = document.createElement("span");
        widgetErr.className = "message-error widget-error";
        widgetErr.textContent = view.last_error;
        card.appendChild(widgetErr);
      }
      const retryWidgetBtn = document.createElement("button");
      retryWidgetBtn.type = "button";
      retryWidgetBtn.className = "button retry-message retry-widget";
      retryWidgetBtn.textContent = t("messages-retry-button");
      retryWidgetBtn.onclick = async () => {
        retryWidgetBtn.disabled = true;
        try {
          await call("conversation.retry", { message_id: view.widget_id });
          await opts.refresh?.();
        } catch {
          retryWidgetBtn.disabled = false;
        }
      };
      card.appendChild(retryWidgetBtn);
    }
  }

  if (ownAnswer && ownAnswer.state !== "delivered") {
    const ansStateSpan = document.createElement("span");
    ansStateSpan.className = `message-state answer-state state-${ownAnswer.state}`;
    ansStateSpan.textContent = messageStateWord(ownAnswer.state);
    card.appendChild(ansStateSpan);

    if (ownAnswer.state === "failed") {
      if (ownAnswer.last_error) {
        const ansErr = document.createElement("span");
        ansErr.className = "message-error answer-error";
        ansErr.textContent = ownAnswer.last_error;
        card.appendChild(ansErr);
      }
      const retryAnsBtn = document.createElement("button");
      retryAnsBtn.type = "button";
      retryAnsBtn.className = "button retry-message retry-answer";
      retryAnsBtn.textContent = t("messages-retry-button");
      retryAnsBtn.onclick = async () => {
        retryAnsBtn.disabled = true;
        try {
          await call("conversation.retry", { message_id: ownAnswer.message_id });
          await opts.refresh?.();
        } catch {
          retryAnsBtn.disabled = false;
        }
      };
      card.appendChild(retryAnsBtn);
    }
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

    const labelIds = new Map<string, string>();
    for (const f of def.fields) {
      labelIds.set(f.name, fieldLabel(def, f, isKnown));
      if (f.choices) {
        for (const c of f.choices) {
          labelIds.set(`${f.name}:${c.code}`, c.text);
        }
      }
    }

    const nonTextFields: FieldDef[] = def.fields
      .filter((f) => f.type !== "text" && f.type !== "long-text")
      .map((f) => ({
        name: f.name,
        type: f.type,
        required: f.required,
        max_len: f.max_len,
        min: f.min,
        max: f.max,
        choices: f.choices?.map((c) => c.code),
      }));

    const textFields = def.fields.filter((f) => f.type === "text" || f.type === "long-text");

    for (const ans of view.answers) {
      const li = document.createElement("li");
      li.className = "widget-answer-item";

      const authorName = opts.nameOf ? opts.nameOf(ans.author) : ans.author;
      const authorSpan = document.createElement("strong");
      authorSpan.className = "answer-author";
      authorSpan.textContent = authorName;
      li.appendChild(authorSpan);

      const nonTextData: Record<string, unknown> = {};
      for (const f of nonTextFields) {
        if (ans.data[f.name] !== undefined) {
          nonTextData[f.name] = ans.data[f.name];
        }
      }

      if (nonTextFields.length > 0 && Object.keys(nonTextData).length > 0) {
        const schema: SchemaDef = {
          schema: def.type,
          fields: nonTextFields,
        };
        li.appendChild(
          fieldView({
            ext: def.extension,
            schema,
            data: nonTextData,
            labelIds,
          }),
        );
      }

      for (const tf of textFields) {
        if (ans.data[tf.name] !== undefined) {
          const val = String(ans.data[tf.name]);
          const p = document.createElement("p");
          p.className = `answer-text-line answer-${tf.type}`;
          const lbl = labelIds.get(tf.name) ?? tf.name;
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
