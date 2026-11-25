import { messageLang, messageText } from "../chat/message_text";
import { widgetNameId, widgetType } from "../extensions/registry";
import { codeWord, t } from "../i18n/i18n";
import { call } from "../rpc";
import { messageStateWord } from "../screens/messages";
import type { WidgetRenderOptions, WidgetView } from "./view";

export function renderPoll(view: WidgetView, opts: WidgetRenderOptions): HTMLElement {
  const card = document.createElement("div");
  card.className = "widget-card widget-poll";
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

  const votesNotice = document.createElement("p");
  votesNotice.className = "widget-votes-visible";
  votesNotice.textContent = t("widget-votes-visible");
  card.appendChild(votesNotice);

  const choiceField = def.fields.find((f) => f.type === "choice") ?? def.fields[0];
  const fieldName = choiceField ? choiceField.name : "option";
  const choices = choiceField?.choices ?? [];

  const ownAnswer = view.answers.find((a) => a.direction === "outgoing");
  const ownChoiceCode = ownAnswer ? (ownAnswer.data?.[fieldName] as string | undefined) : undefined;
  const ownChoice = choices.find((c) => c.code === ownChoiceCode);

  if (ownChoice) {
    const choiceP = document.createElement("p");
    choiceP.className = "widget-poll-your-choice";
    choiceP.textContent = t("widget-poll-your-choice", { choice: ownChoice.text });
    card.appendChild(choiceP);
  }

  const optionsDiv = document.createElement("div");
  optionsDiv.className = "widget-poll-options";

  for (const c of choices) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "button widget-poll-option-btn";
    if (c.code === ownChoiceCode) {
      btn.classList.add("selected");
    }

    const textSpan = document.createElement("span");
    textSpan.className = "poll-option-text";
    textSpan.textContent = c.text;
    textSpan.setAttribute("lang", messageLang(view.languages ?? []));

    const count = view.counts.find((cnt) => cnt.field === fieldName && cnt.choice === c.code)?.count ?? 0;
    const countSpan = document.createElement("span");
    countSpan.className = "poll-option-count";
    countSpan.textContent = t("widget-poll-count", { count });

    btn.append(textSpan, " ", countSpan);

    btn.onclick = async () => {
      if (c.code === ownChoiceCode) {
        return;
      }
      btn.disabled = true;
      try {
        await call("widget.answer", {
          conversation: opts.conversation,
          widget_id: view.widget_id,
          data: { [fieldName]: c.code },
        });
        await opts.refresh?.();
      } catch {
        btn.disabled = false;
      }
    };

    optionsDiv.appendChild(btn);
  }

  card.appendChild(optionsDiv);

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

  return card;
}
