import { RECORD_LANGUAGES, languageName } from "./text_lang";

export interface LanguageListLabels {
  add: string;
  moveUp: string;
  remove: string;
}

export interface LanguageList {
  /** A <div> that holds the <ul> of rows and the "add" <select>. */
  root: HTMLElement;
  /** The chosen tags, in order. */
  value(): string[];
  /** Replaces the chosen tags and draws the rows again. */
  set(tags: string[]): void;
}

/// An ordered list of language tags from RECORD_LANGUAGES, at most `max`:
/// each row has the language name, "move up" (not on the first row) and
/// "remove"; the "add" <select> offers the languages not yet chosen and is
/// disabled at `max`. Every button has type="button". The class names are
/// `${prefix}-languages-list`, `${prefix}-language-row`,
/// `${prefix}-language-name`, `${prefix}-language-move-up`,
/// `${prefix}-language-remove` and `${prefix}-languages-add`.
export function languageList(
  prefix: string,
  max: number,
  initial: string[],
  labels: LanguageListLabels,
  groupLabel: string,
): LanguageList {
  let tags = [...initial];

  const root = document.createElement("div");
  root.setAttribute("role", "group");
  root.setAttribute("aria-label", groupLabel);

  const listContainer = document.createElement("ul");
  listContainer.className = `${prefix}-languages-list`;
  root.appendChild(listContainer);

  const addSelect = document.createElement("select");
  addSelect.className = `${prefix}-languages-add`;
  addSelect.setAttribute("aria-label", groupLabel);
  root.appendChild(addSelect);

  function renderRows() {
    listContainer.replaceChildren();
    for (let idx = 0; idx < tags.length; idx++) {
      const tag = tags[idx];
      const li = document.createElement("li");
      li.className = `${prefix}-language-row`;

      const nameSpan = document.createElement("span");
      nameSpan.className = `${prefix}-language-name`;
      nameSpan.textContent = languageName(tag);
      li.appendChild(nameSpan);

      if (idx > 0) {
        const upBtn = document.createElement("button");
        upBtn.type = "button";
        upBtn.className = `button ${prefix}-language-move-up`;
        upBtn.textContent = labels.moveUp;
        upBtn.onclick = () => {
          const temp = tags[idx - 1];
          tags[idx - 1] = tags[idx];
          tags[idx] = temp;
          renderRows();
        };
        li.appendChild(upBtn);
      }

      const rmBtn = document.createElement("button");
      rmBtn.type = "button";
      rmBtn.className = `button ${prefix}-language-remove`;
      rmBtn.textContent = labels.remove;
      rmBtn.onclick = () => {
        tags.splice(idx, 1);
        renderRows();
      };
      li.appendChild(rmBtn);

      listContainer.appendChild(li);
    }

    addSelect.replaceChildren();
    const defaultOpt = document.createElement("option");
    defaultOpt.value = "";
    defaultOpt.textContent = labels.add;
    addSelect.appendChild(defaultOpt);

    for (const tag of RECORD_LANGUAGES) {
      if (!tags.includes(tag)) {
        const opt = document.createElement("option");
        opt.value = tag;
        opt.textContent = languageName(tag);
        addSelect.appendChild(opt);
      }
    }
    addSelect.disabled = tags.length >= max;
  }

  addSelect.onchange = () => {
    if (addSelect.value && tags.length < max) {
      tags.push(addSelect.value);
      renderRows();
      addSelect.value = "";
    }
  };

  renderRows();

  return {
    root,
    value() {
      return [...tags];
    },
    set(newTags: string[]) {
      tags = [...newTags];
      renderRows();
    },
  };
}
