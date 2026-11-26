import { messageLang, messageText } from "../chat/message_text";
import { errText } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import { stateAndRetry, widgetHeading } from "./parts";
import type { WidgetRenderOptions, WidgetView } from "./view";

export function renderPoll(view: WidgetView, opts: WidgetRenderOptions): HTMLElement {
  const card = document.createElement("div");
  card.className = "widget-card widget-poll";
  card.dataset.widgetId = view.widget_id;

  const def = view.definition;
  if (!def) {
    return card;
  }

  widgetHeading(card, view);

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
      const allBtns = optionsDiv.querySelectorAll<HTMLButtonElement>(".widget-poll-option-btn");
      allBtns.forEach((b) => (b.disabled = true));
      try {
        await call("widget.answer", {
          conversation: opts.conversation,
          widget_id: view.widget_id,
          data: { [fieldName]: c.code },
          languages: opts.languages ? opts.languages() : [],
        });
        const errP = card.querySelector(".widget-answer-error");
        errP?.remove();
        await opts.refresh?.();
      } catch (err) {
        allBtns.forEach((b) => (b.disabled = false));
        let errP = card.querySelector<HTMLParagraphElement>(".widget-answer-error");
        if (!errP) {
          errP = document.createElement("p");
          errP.className = "widget-answer-error error-text";
          card.appendChild(errP);
        }
        errP.textContent = t("widget-answer-failed", { detail: errText(err) });
      }
    };

    optionsDiv.appendChild(btn);
  }

  card.appendChild(optionsDiv);

  if (ownAnswer) {
    const isRestored = opts.isRestored?.(ownAnswer.message_id) ?? false;
    stateAndRetry(card, ownAnswer.state, ownAnswer.last_error, ownAnswer.message_id, "answer", opts.refresh, isRestored);
  }

  return card;
}
