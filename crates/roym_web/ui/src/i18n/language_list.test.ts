import { describe, expect, it } from "vitest";
import { languageList } from "./language_list";

describe("languageList component", () => {
  const labels = {
    add: "Add a language",
    moveUp: "Move up",
    remove: "Remove",
  };

  it("starts with initial and value() gives tags in order", () => {
    const list = languageList("messages", 3, ["en"], labels, "Message languages");
    expect(list.value()).toEqual(["en"]);
    const rows = list.root.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("English");
  });

  it("move up on row 2 makes it row 1; row 1 has no move up", () => {
    const list = languageList("messages", 3, ["en", "hi"], labels, "Message languages");
    expect(list.value()).toEqual(["en", "hi"]);

    let rows = list.root.querySelectorAll(".messages-language-row");
    expect(rows[0].querySelector(".messages-language-move-up")).toBeNull();

    const upBtn = rows[1].querySelector<HTMLButtonElement>(".messages-language-move-up");
    expect(upBtn).not.toBeNull();
    upBtn?.click();

    expect(list.value()).toEqual(["hi", "en"]);
    rows = list.root.querySelectorAll(".messages-language-row");
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("Hindi");
    expect(rows[1].querySelector(".messages-language-name")?.textContent).toBe("English");
  });

  it("remove drops the row; removing every row gives []", () => {
    const list = languageList("messages", 3, ["en", "hi"], labels, "Message languages");
    let rows = list.root.querySelectorAll(".messages-language-row");
    const rmBtn1 = rows[0].querySelector<HTMLButtonElement>(".messages-language-remove");
    rmBtn1?.click();

    expect(list.value()).toEqual(["hi"]);
    rows = list.root.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(1);

    const rmBtn2 = rows[0].querySelector<HTMLButtonElement>(".messages-language-remove");
    rmBtn2?.click();

    expect(list.value()).toEqual([]);
    expect(list.root.querySelectorAll(".messages-language-row")).toHaveLength(0);
  });

  it("add list offers only RECORD_LANGUAGES not yet chosen, and is disabled at max", () => {
    const list3 = languageList("messages", 3, ["en", "hi"], labels, "Message languages");
    const addSel3 = list3.root.querySelector<HTMLSelectElement>(".messages-languages-add")!;
    expect(addSel3).not.toBeNull();

    const opts3 = Array.from(addSel3.options).map((o) => o.value);
    expect(opts3).toContain("");
    expect(opts3).not.toContain("en");
    expect(opts3).not.toContain("hi");
    expect(opts3).toContain("ta");
    expect(addSel3.disabled).toBe(false);

    // Add ta -> now 3, should be disabled
    addSel3.value = "ta";
    addSel3.dispatchEvent(new Event("change"));
    expect(list3.value()).toEqual(["en", "hi", "ta"]);
    expect(addSel3.disabled).toBe(true);

    // Check with max 8
    const list8 = languageList("profile", 8, ["en"], labels, "Languages");
    const addSel8 = list8.root.querySelector<HTMLSelectElement>(".profile-languages-add")!;
    expect(addSel8.disabled).toBe(false);
    const tagsToAdd = ["bn", "gu", "kn", "ml", "mr", "or", "pa"];
    for (const tag of tagsToAdd) {
      addSel8.value = tag;
      addSel8.dispatchEvent(new Event("change"));
    }
    expect(list8.value()).toHaveLength(8);
    expect(addSel8.disabled).toBe(true);
  });

  it("every button has type='button'; a click inside a <form> does not submit it", () => {
    const form = document.createElement("form");
    let submitted = false;
    form.onsubmit = (e) => {
      e.preventDefault();
      submitted = true;
    };
    const list = languageList("messages", 3, ["en", "hi"], labels, "Message languages");
    form.appendChild(list.root);
    document.body.appendChild(form);

    const buttons = list.root.querySelectorAll("button");
    expect(buttons.length).toBeGreaterThan(0);
    for (const btn of buttons) {
      expect(btn.type).toBe("button");
      btn.click();
      expect(submitted).toBe(false);
    }
    document.body.removeChild(form);
  });

  it("set(['ta', 'en']) draws two rows in that order", () => {
    const list = languageList("messages", 3, ["en"], labels, "Message languages");
    list.set(["ta", "en"]);
    expect(list.value()).toEqual(["ta", "en"]);
    const rows = list.root.querySelectorAll(".messages-language-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".messages-language-name")?.textContent).toBe("Tamil");
    expect(rows[1].querySelector(".messages-language-name")?.textContent).toBe("English");
  });

  it("class names follow prefix", () => {
    const list = languageList("custom", 3, ["en"], labels, "Custom");
    expect(list.root.querySelector(".custom-languages-list")).not.toBeNull();
    expect(list.root.querySelector(".custom-languages-add")).not.toBeNull();
    expect(list.root.querySelector(".custom-language-row")).not.toBeNull();
    expect(list.root.querySelector(".custom-language-name")).not.toBeNull();
    expect(list.root.querySelector(".custom-language-remove")).not.toBeNull();
  });

  it("accessibility: root has role='group' and aria-label; add select has aria-label", () => {
    const list = languageList("messages", 3, ["en"], labels, "Message languages");
    expect(list.root.getAttribute("role")).toBe("group");
    expect(list.root.getAttribute("aria-label")).toBe("Message languages");
    const addSel = list.root.querySelector(".messages-languages-add");
    expect(addSel?.getAttribute("aria-label")).toBe("Message languages");
  });
});
