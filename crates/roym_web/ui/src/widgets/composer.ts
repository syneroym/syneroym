import { errText } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import { pollDefinition } from "./view";

export interface PollComposerOptions {
  conversation: string;
  languages: () => string[];
  onSent?: () => Promise<void> | void;
}

export interface PollComposer {
  button: HTMLButtonElement;
  form: HTMLElement;
  toggle: () => void;
  reset: () => void;
}

export function pollComposer(opts: PollComposerOptions): PollComposer {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "button button-secondary toggle-poll-button";
  button.textContent = t("widget-poll-button");

  const form = document.createElement("div");
  form.className = "poll-composer-form";
  form.style.display = "none";

  const questionInput = document.createElement("input");
  questionInput.type = "text";
  questionInput.className = "widget-poll-question-input";
  questionInput.placeholder = t("widget-poll-question");

  const optionsContainer = document.createElement("div");
  optionsContainer.className = "widget-poll-options-container";

  const hintP = document.createElement("p");
  hintP.className = "widget-poll-composer-hint";

  const addOptionBtn = document.createElement("button");
  addOptionBtn.type = "button";
  addOptionBtn.className = "button-secondary widget-poll-add-option";
  addOptionBtn.textContent = t("widget-poll-add-option");

  const sendBtn = document.createElement("button");
  sendBtn.type = "button";
  sendBtn.className = "button widget-poll-send-btn";
  sendBtn.textContent = t("widget-poll-send");

  let optionValues: string[] = ["", ""];

  function validate(): { valid: boolean; reason: string } {
    const q = questionInput.value.trim();
    if (!q) {
      return { valid: false, reason: "" };
    }
    const nonBlank = optionValues.map((v) => v.trim()).filter(Boolean);
    if (nonBlank.length < 2 || optionValues.some((v) => !v.trim())) {
      return { valid: false, reason: t("widget-poll-needs-two") };
    }
    const set = new Set<string>();
    for (const opt of nonBlank) {
      const lower = opt.toLowerCase();
      if (set.has(lower)) {
        return { valid: false, reason: t("widget-poll-options-differ") };
      }
      set.add(lower);
    }
    return { valid: true, reason: "" };
  }

  function updateFormState() {
    optionsContainer.replaceChildren();
    optionValues.forEach((val, idx) => {
      const row = document.createElement("div");
      row.className = "widget-poll-option-row";

      const optInput = document.createElement("input");
      optInput.type = "text";
      optInput.className = "widget-poll-option-input";
      optInput.placeholder = t("widget-poll-option", { n: idx + 1 });
      optInput.value = val;
      optInput.oninput = () => {
        optionValues[idx] = optInput.value;
        const res = validate();
        sendBtn.disabled = !res.valid;
        hintP.textContent = res.reason;
      };

      const removeBtn = document.createElement("button");
      removeBtn.type = "button";
      removeBtn.className = "button-small widget-poll-remove-option";
      removeBtn.textContent = t("widget-poll-remove-option");
      if (optionValues.length <= 2) {
        removeBtn.disabled = true;
      }
      removeBtn.onclick = () => {
        if (optionValues.length > 2) {
          optionValues.splice(idx, 1);
          updateFormState();
        }
      };

      row.append(optInput, removeBtn);
      optionsContainer.appendChild(row);
    });

    addOptionBtn.disabled = optionValues.length >= 12;

    const res = validate();
    sendBtn.disabled = !res.valid;
    hintP.textContent = res.reason;
  }

  addOptionBtn.onclick = () => {
    if (optionValues.length < 12) {
      optionValues.push("");
      updateFormState();
    }
  };

  questionInput.oninput = () => {
    const res = validate();
    sendBtn.disabled = !res.valid;
    hintP.textContent = res.reason;
  };

  sendBtn.onclick = async () => {
    const res = validate();
    if (!res.valid) return;
    sendBtn.disabled = true;
    try {
      const q = questionInput.value.trim();
      const optsList = optionValues.map((v) => v.trim());
      const def = pollDefinition(q, optsList);
      await call("widget.send", {
        conversation: opts.conversation,
        widget: def,
        languages: opts.languages(),
      });
      reset();
      form.style.display = "none";
      await opts.onSent?.();
    } catch (err) {
      hintP.textContent = t("widget-send-failed", { detail: errText(err) });
      sendBtn.disabled = false;
    }
  };

  function reset() {
    questionInput.value = "";
    optionValues = ["", ""];
    updateFormState();
  }

  function toggle() {
    if (form.style.display === "none") {
      form.style.display = "block";
    } else {
      form.style.display = "none";
    }
  }

  button.onclick = toggle;

  form.append(questionInput, optionsContainer, addOptionBtn, hintP, sendBtn);
  updateFormState();

  return { button, form, toggle, reset };
}
