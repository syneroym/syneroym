import { t } from "../i18n/i18n";
import {
  CATEGORY_GROUPS,
  categoryGroupOf,
  categoryLabel,
  isTaxonomyCategory,
  matchCategories,
} from "./taxonomy";

export interface CategoryPickerOptions {
  className: string;
  max: number;
  selected?: readonly string[];
}

export interface CategoryPicker {
  root: HTMLElement;
  codes(): string[];
  clear(): void;
}

export function categoryPicker(opts: CategoryPickerOptions): CategoryPicker {
  const root = document.createElement("div");
  root.className = `category-picker ${opts.className}`;

  const chosenList = document.createElement("ul");
  chosenList.className = "picker-chosen";
  root.appendChild(chosenList);

  const filter = document.createElement("input");
  filter.type = "text";
  filter.className = "picker-filter";
  filter.placeholder = t("category-picker-filter-placeholder");
  root.appendChild(filter);

  const message = document.createElement("div");
  message.className = "picker-message";
  message.setAttribute("aria-live", "polite");
  root.appendChild(message);

  const options = document.createElement("div");
  options.className = "picker-options";
  options.hidden = true;
  root.appendChild(options);

  function openOptions() {
    options.hidden = false;
  }

  function closeOptions() {
    options.hidden = true;
  }

  root.addEventListener("focusin", () => {
    openOptions();
  });

  root.addEventListener("focusout", (e: FocusEvent) => {
    const next = e.relatedTarget as Node | null;
    if (!root.contains(next) && !filter.value.trim()) {
      closeOptions();
    }
  });

  root.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      closeOptions();
      filter.blur();
    }
  });

  const selectedCodes: string[] = [];
  if (opts.selected) {
    for (const code of opts.selected) {
      if (!selectedCodes.includes(code)) {
        selectedCodes.push(code);
      }
    }
  }

  function renderChips() {
    chosenList.replaceChildren();
    for (const code of selectedCodes) {
      const li = document.createElement("li");
      li.className = "picker-chip";
      li.dataset.code = code;

      const labelText = categoryLabel(code);
      const span = document.createElement("span");
      span.textContent = labelText;
      li.appendChild(span);

      const removeBtn = document.createElement("button");
      removeBtn.type = "button";
      removeBtn.className = "picker-chip-remove";
      removeBtn.setAttribute(
        "aria-label",
        t("category-picker-remove", { label: labelText }),
      );
      removeBtn.textContent = "×";
      removeBtn.addEventListener("click", () => {
        removeCode(code);
      });
      li.appendChild(removeBtn);

      chosenList.appendChild(li);
    }
  }

  function pick(code: string) {
    if (selectedCodes.length >= opts.max || selectedCodes.includes(code)) {
      return;
    }
    selectedCodes.push(code);
    filter.value = "";
    renderChips();
    renderOptions();
  }

  function removeCode(code: string) {
    const idx = selectedCodes.indexOf(code);
    if (idx !== -1) {
      selectedCodes.splice(idx, 1);
      renderChips();
      renderOptions();
    }
  }

  function renderOptions() {
    message.textContent = "";
    options.replaceChildren();

    const atMax = selectedCodes.length >= opts.max;
    if (atMax) {
      message.textContent = t("category-picker-full", { max: opts.max });
    }

    const text = filter.value.trim();
    if (!text) {
      for (const group of CATEGORY_GROUPS) {
        const unchosen = group.children.filter((c) => !selectedCodes.includes(c));
        if (unchosen.length > 0) {
          const heading = document.createElement("div");
          heading.className = "picker-group";
          heading.textContent = categoryLabel(group.code);
          options.appendChild(heading);

          for (const childCode of unchosen) {
            const btn = document.createElement("button");
            btn.type = "button";
            btn.className = "picker-option";
            btn.dataset.code = childCode;
            btn.textContent = categoryLabel(childCode);
            btn.disabled = atMax;
            btn.addEventListener("click", () => {
              pick(childCode);
            });
            options.appendChild(btn);
          }
        }
      }
      return;
    }

    const matched = matchCategories(text).filter((c) => !selectedCodes.includes(c));
    const grouped = new Map<string, string[]>();
    for (const c of matched) {
      const parent = categoryGroupOf(c);
      if (parent) {
        const list = grouped.get(parent);
        if (list) {
          list.push(c);
        } else {
          grouped.set(parent, [c]);
        }
      }
    }

    for (const group of CATEGORY_GROUPS) {
      const children = grouped.get(group.code);
      if (children && children.length > 0) {
        const heading = document.createElement("div");
        heading.className = "picker-group";
        heading.textContent = categoryLabel(group.code);
        options.appendChild(heading);

        for (const childCode of children) {
          const btn = document.createElement("button");
          btn.type = "button";
          btn.className = "picker-option";
          btn.dataset.code = childCode;
          btn.textContent = categoryLabel(childCode);
          btn.disabled = atMax;
          btn.addEventListener("click", () => {
            pick(childCode);
          });
          options.appendChild(btn);
        }
      }
    }

    const codeCandidate = text.toLowerCase();
    const isValidCode = /^[a-z0-9-]{1,64}$/.test(codeCandidate);
    const isTaxonomy = isTaxonomyCategory(codeCandidate);
    const isAlreadyChosen = selectedCodes.includes(codeCandidate);
    let hasOwnCode = false;

    if (isValidCode && !isTaxonomy && !isAlreadyChosen) {
      hasOwnCode = true;
      const ownBtn = document.createElement("button");
      ownBtn.type = "button";
      ownBtn.className = "picker-option picker-own-code";
      ownBtn.dataset.code = codeCandidate;
      ownBtn.textContent = t("category-picker-own-code", { code: codeCandidate });
      ownBtn.disabled = atMax;
      ownBtn.addEventListener("click", () => {
        pick(codeCandidate);
      });
      options.appendChild(ownBtn);
    }

    if (matched.length === 0 && !hasOwnCode) {
      message.textContent = t("category-picker-no-match");
    }
  }

  filter.addEventListener("input", () => {
    openOptions();
    renderOptions();
  });

  filter.addEventListener("keydown", (e) => {
    if (e.key === "Tab" && !filter.value.trim()) {
      closeOptions();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const visibleButtons = options.querySelectorAll<HTMLButtonElement>(
        "button.picker-option:not(:disabled)",
      );
      if (visibleButtons.length === 1) {
        visibleButtons[0].click();
      }
    }
  });

  renderChips();
  renderOptions();

  return {
    root,
    codes(): string[] {
      return Array.from(new Set(selectedCodes));
    },
    clear() {
      selectedCodes.length = 0;
      filter.value = "";
      closeOptions();
      renderChips();
      renderOptions();
    },
  };
}
