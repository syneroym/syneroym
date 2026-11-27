import type { ServiceLocationKind } from "../listings/editor";
import { serviceLocationEditorLabel } from "../listings/labels";
import { t } from "../i18n/i18n";
import { currencyMinorExponent, minorToDecimal, MoneyInputError, toMinorUnits } from "../money";
import { parseDateInput, toDateInputValue } from "../screens/statement";
import type { CommitmentRowFields } from "../screens/quote_terms";

export interface RowHandles {
  element: HTMLElement;
  read: () => CommitmentRowFields | undefined;
  setDeclaredValue: (val: string) => void;
  isFirstAuthorService: boolean;
  isFirstMoney: boolean;
  userModifiedValue: boolean;
}

function buildLocationSection(
  initial: Partial<CommitmentRowFields>,
  onChanged: () => void,
) {
  const locationWrap = document.createElement("div");
  locationWrap.className = "quote-row-location-wrap";
  const whereSelect = document.createElement("select");
  const optReceiver = document.createElement("option");
  optReceiver.value = "at-receiver";
  optReceiver.textContent = serviceLocationEditorLabel("at-receiver");
  const optGiver = document.createElement("option");
  optGiver.value = "at-giver";
  optGiver.textContent = serviceLocationEditorLabel("at-giver");
  const optRem = document.createElement("option");
  optRem.value = "remote";
  optRem.textContent = serviceLocationEditorLabel("remote");
  whereSelect.append(optReceiver, optGiver, optRem);
  if (initial.where) whereSelect.value = initial.where;

  const addressWrap = document.createElement("div");
  addressWrap.className = "quote-address-wrap";
  const addressNotice = document.createElement("p");
  addressNotice.className = "address-disclosure-notice";
  addressNotice.textContent = t("quote-address-disclosure-notice");
  const addressInput = document.createElement("input");
  addressInput.placeholder = t("quote-address-placeholder");
  addressInput.className = "quote-address-input";
  if (initial.address) addressInput.value = initial.address;
  addressWrap.append(addressNotice, addressInput);

  whereSelect.onchange = () => {
    addressWrap.style.display = whereSelect.value === "at-receiver" ? "block" : "none";
    onChanged();
  };
  addressWrap.style.display = whereSelect.value === "at-receiver" ? "block" : "none";
  locationWrap.append(whereSelect, addressWrap);

  return { locationWrap, whereSelect, addressInput };
}

function buildWindowSection(
  initial: Partial<CommitmentRowFields>,
  onChanged: () => void,
) {
  const windowWrap = document.createElement("div");
  windowWrap.className = "quote-row-window-wrap";
  windowWrap.style.display = "none";
  const whenLabel = document.createElement("label");
  whenLabel.className = "quote-row-when-label";
  whenLabel.textContent = t("deal-row-when");
  const fromInput = document.createElement("input");
  fromInput.className = "quote-row-from-input";
  fromInput.type = "date";
  fromInput.placeholder = t("deal-from-secs-placeholder");
  const toInput = document.createElement("input");
  toInput.className = "quote-row-to-input";
  toInput.type = "date";
  toInput.placeholder = t("deal-to-secs-placeholder");
  if (initial.window) {
    if (initial.window.earliest_secs) {
      fromInput.value = toDateInputValue(new Date(initial.window.earliest_secs * 1000));
    }
    if (initial.window.latest_secs) {
      toInput.value = toDateInputValue(new Date(initial.window.latest_secs * 1000));
    }
  }
  fromInput.oninput = onChanged;
  toInput.oninput = onChanged;
  windowWrap.append(whenLabel, fromInput, toInput);

  return { windowWrap, fromInput, toInput };
}

function buildMoneySection(
  initial: Partial<CommitmentRowFields>,
  isFirstMoney: boolean,
  getCurrency: () => string,
  onChanged: () => void,
  updateValueHint: () => void,
) {
  const moneyWrap = document.createElement("div");
  moneyWrap.className = "quote-row-money-wrap";
  let currencyInput: HTMLInputElement | undefined;
  if (isFirstMoney) {
    currencyInput = document.createElement("input");
    currencyInput.className = "quote-currency-input";
    currencyInput.placeholder = t("quote-currency-placeholder");
    currencyInput.defaultValue = getCurrency();
    currencyInput.oninput = () => {
      updateValueHint();
      onChanged();
    };
    moneyWrap.appendChild(currencyInput);
  }

  const payeeInput = document.createElement("input");
  payeeInput.placeholder = t("quote-payee-placeholder");
  if (initial.payee) payeeInput.value = initial.payee;
  payeeInput.oninput = onChanged;

  const taxInput = document.createElement("input");
  taxInput.className = "quote-tax-input";
  taxInput.placeholder = t("quote-tax-placeholder");
  if (initial.taxMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    taxInput.value = minorToDecimal(initial.taxMinor, exp);
  }
  taxInput.oninput = onChanged;

  const feesInput = document.createElement("input");
  feesInput.className = "quote-fees-input";
  feesInput.placeholder = t("quote-fees-placeholder");
  if (initial.feesMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    feesInput.value = minorToDecimal(initial.feesMinor, exp);
  }
  feesInput.oninput = onChanged;

  moneyWrap.append(payeeInput, taxInput, feesInput);
  return { moneyWrap, currencyInput, payeeInput, taxInput, feesInput };
}

function readWindowData(fromInput: HTMLInputElement, toInput: HTMLInputElement): {
  window?: { earliest_secs: number; latest_secs: number };
  error?: string;
} {
  const fromVal = fromInput.value.trim();
  const toVal = toInput.value.trim();
  if (!fromVal && !toVal) return {};
  if (!fromVal || !toVal) {
    return { error: t("deal-error-window-required") };
  }
  const fromD = parseDateInput(fromVal);
  const toD = parseDateInput(toVal);
  if (!fromD || !toD || fromD.getTime() > toD.getTime()) {
    return { error: t("deal-error-window-required") };
  }
  const earliest_secs = Math.floor(fromD.getTime() / 1000);
  const nextDay = new Date(toD.getFullYear(), toD.getMonth(), toD.getDate() + 1, 0, 0, 0, 0);
  const latest_secs = Math.floor(nextDay.getTime() / 1000) - 1;
  return { window: { earliest_secs, latest_secs } };
}

function readRowData(
  giverSelect: HTMLSelectElement,
  kindSelect: HTMLSelectElement,
  valueInput: HTMLInputElement,
  descInput: HTMLInputElement,
  whereSelect: HTMLSelectElement,
  addressInput: HTMLInputElement,
  fromInput: HTMLInputElement,
  toInput: HTMLInputElement,
  payeeInput: HTMLInputElement,
  taxInput: HTMLInputElement,
  feesInput: HTMLInputElement,
  getCurrency: () => string,
): CommitmentRowFields {
  const giver = giverSelect.value as "quote-author" | "counterparty";
  const kind = kindSelect.value as "service" | "goods" | "money";
  const exp = currencyMinorExponent(getCurrency()) ?? 2;

  let declaredValueMinor = 0;
  let error: string | undefined;
  if (valueInput.value.trim()) {
    try {
      declaredValueMinor = toMinorUnits(valueInput.value.trim(), exp) ?? 0;
    } catch (e) {
      error = e instanceof MoneyInputError ? e.message : String(e);
    }
  }

  const res: CommitmentRowFields = {
    giver,
    kind,
    description: descInput.value.trim(),
    declaredValueMinor,
    ...(error ? { error } : {}),
  };

  if (kind === "service" && giver === "quote-author") {
    res.where = whereSelect.value as ServiceLocationKind;
    if (addressInput.value.trim()) res.address = addressInput.value.trim();
  }

  const win = readWindowData(fromInput, toInput);
  if (win.error && !res.error) res.error = win.error;
  if (win.window) res.window = win.window;

  if (kind === "money") {
    res.payee = payeeInput.value.trim();
    if (taxInput.value.trim()) {
      try {
        res.taxMinor = toMinorUnits(taxInput.value.trim(), exp) ?? 0;
      } catch (e) {
        if (!res.error && e instanceof MoneyInputError) res.error = e.message;
      }
    }
    if (feesInput.value.trim()) {
      try {
        res.feesMinor = toMinorUnits(feesInput.value.trim(), exp) ?? 0;
      } catch (e) {
        if (!res.error && e instanceof MoneyInputError) res.error = e.message;
      }
    }
  }

  return res;
}

function buildSelectors(
  initial: Partial<CommitmentRowFields>,
  index: number,
) {
  const giverSelect = document.createElement("select");
  giverSelect.className = "quote-row-giver-select";
  const optMe = document.createElement("option");
  optMe.value = "quote-author";
  optMe.textContent = t("deal-giver-me");
  const optThem = document.createElement("option");
  optThem.value = "counterparty";
  optThem.textContent = t("deal-giver-them");
  giverSelect.append(optMe, optThem);
  giverSelect.value = initial.giver || (index === 0 ? "quote-author" : "counterparty");

  const kindSelect = document.createElement("select");
  kindSelect.className = "quote-row-kind-select";
  const optService = document.createElement("option");
  optService.value = "service";
  optService.textContent = t("deal-kind-service");
  const optGoods = document.createElement("option");
  optGoods.value = "goods";
  optGoods.textContent = t("deal-kind-goods");
  const optMoney = document.createElement("option");
  optMoney.value = "money";
  optMoney.textContent = t("deal-kind-money");
  kindSelect.append(optService, optGoods, optMoney);
  kindSelect.value = initial.kind || (index === 0 ? "service" : "money");

  return { giverSelect, kindSelect };
}

function buildDescAndValueInputs(
  initial: Partial<CommitmentRowFields>,
  isFirstAuthor: boolean,
  isFirstMoney: boolean,
  getCurrency: () => string,
  onChanged: () => void,
  markUserModified: () => void,
) {
  const descInput = document.createElement("input");
  descInput.className = "quote-row-desc-input";
  if (isFirstAuthor) {
    descInput.classList.add("quote-give-input");
    descInput.placeholder = t("quote-give-placeholder");
    descInput.setAttribute("aria-label", t("quote-give-label"));
  } else {
    descInput.placeholder = t("quote-give-placeholder");
  }
  if (initial.description) descInput.value = initial.description;

  const valueWrap = document.createElement("div");
  valueWrap.className = "quote-row-value-wrap";
  const valueInput = document.createElement("input");
  valueInput.className = "quote-row-value-input";
  if (isFirstMoney) {
    valueInput.classList.add("quote-amount-input");
  }
  valueInput.placeholder = t("quote-amount-placeholder");
  if (initial.declaredValueMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    valueInput.value = minorToDecimal(initial.declaredValueMinor, exp);
  }
  valueInput.oninput = () => {
    markUserModified();
    onChanged();
  };

  const valueHint = document.createElement("p");
  valueHint.className = "quote-row-value-hint";
  const updateValueHint = () => {
    const curr = getCurrency() || "INR";
    valueHint.textContent = t("deal-row-value-hint", { currency: curr });
  };
  updateValueHint();
  valueWrap.append(valueInput, valueHint);

  return { descInput, valueWrap, valueInput, valueHint, updateValueHint };
}

function applyRowVisibility(
  row: HTMLElement,
  kind: string,
  giver: string,
  descInput: HTMLElement,
  valueHint: HTMLElement,
  locationWrap: HTMLElement,
  windowWrap: HTMLElement,
  moneyWrap: HTMLElement,
  payeeInput: HTMLElement,
  whereSelect: HTMLElement,
  updateValueHint: () => void,
) {
  if (kind === "money") {
    descInput.style.display = "none";
    valueHint.style.display = "none";
    locationWrap.remove();
    windowWrap.style.display = "none";
    moneyWrap.style.display = "block";
    payeeInput.classList.add("quote-payee-input");
  } else {
    descInput.style.display = "block";
    valueHint.style.display = "block";
    updateValueHint();
    moneyWrap.style.display = "none";
    payeeInput.classList.remove("quote-payee-input");
    windowWrap.style.display = "flex";
    if (kind === "service" && giver === "quote-author") {
      if (!row.contains(locationWrap)) {
        row.insertBefore(locationWrap, windowWrap);
      }
      locationWrap.style.display = "block";
      whereSelect.classList.add("quote-where-select");
    } else {
      locationWrap.remove();
      whereSelect.classList.remove("quote-where-select");
    }
  }
}

function wireRowEvents(
  row: HTMLElement,
  kindSelect: HTMLSelectElement,
  giverSelect: HTMLSelectElement,
  descInput: HTMLInputElement,
  valueHint: HTMLElement,
  locationWrap: HTMLElement,
  windowWrap: HTMLElement,
  moneyWrap: HTMLElement,
  payeeInput: HTMLInputElement,
  whereSelect: HTMLSelectElement,
  addressInput: HTMLInputElement,
  updateValueHint: () => void,
  onChanged: () => void,
) {
  const updateVisibility = () => {
    applyRowVisibility(
      row,
      kindSelect.value,
      giverSelect.value,
      descInput,
      valueHint,
      locationWrap,
      windowWrap,
      moneyWrap,
      payeeInput,
      whereSelect,
      updateValueHint,
    );
  };

  giverSelect.onchange = () => {
    updateVisibility();
    onChanged();
  };
  kindSelect.onchange = () => {
    updateVisibility();
    onChanged();
  };
  descInput.oninput = onChanged;
  addressInput.oninput = onChanged;
  updateVisibility();
}

function makeRowReader(
  giverSelect: HTMLSelectElement,
  kindSelect: HTMLSelectElement,
  valueInput: HTMLInputElement,
  descInput: HTMLInputElement,
  whereSelect: HTMLSelectElement,
  addressInput: HTMLInputElement,
  fromInput: HTMLInputElement,
  toInput: HTMLInputElement,
  payeeInput: HTMLInputElement,
  taxInput: HTMLInputElement,
  feesInput: HTMLInputElement,
  getCurrency: () => string,
): () => CommitmentRowFields {
  return () =>
    readRowData(
      giverSelect,
      kindSelect,
      valueInput,
      descInput,
      whereSelect,
      addressInput,
      fromInput,
      toInput,
      payeeInput,
      taxInput,
      feesInput,
      getCurrency,
    );
}

export function createCommitmentRow(
  index: number,
  initial: Partial<CommitmentRowFields>,
  isFirstAuthor: boolean,
  isFirstMoney: boolean,
  getCurrency: () => string,
  onChanged: () => void,
  onRemove: () => void,
): RowHandles {
  const row = document.createElement("div");
  row.className = "quote-commitment-row";
  row.dataset.index = String(index);

  let userModifiedValue = false;
  const { giverSelect, kindSelect } = buildSelectors(initial, index);
  const { descInput, valueWrap, valueInput, valueHint, updateValueHint } = buildDescAndValueInputs(
    initial,
    isFirstAuthor,
    isFirstMoney,
    getCurrency,
    onChanged,
    () => {
      userModifiedValue = true;
    },
  );

  const { locationWrap, whereSelect, addressInput } = buildLocationSection(initial, onChanged);
  const { windowWrap, fromInput, toInput } = buildWindowSection(initial, onChanged);
  const { moneyWrap, payeeInput, taxInput, feesInput } = buildMoneySection(
    initial,
    isFirstMoney,
    getCurrency,
    onChanged,
    updateValueHint,
  );

  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.className = "button quote-row-remove-button";
  removeBtn.textContent = t("deal-remove-line");
  removeBtn.onclick = onRemove;

  row.append(giverSelect, kindSelect, descInput, valueWrap, windowWrap, moneyWrap, removeBtn);

  wireRowEvents(
    row,
    kindSelect,
    giverSelect,
    descInput,
    valueHint,
    locationWrap,
    windowWrap,
    moneyWrap,
    payeeInput,
    whereSelect,
    addressInput,
    updateValueHint,
    onChanged,
  );

  return {
    element: row,
    read: makeRowReader(
      giverSelect,
      kindSelect,
      valueInput,
      descInput,
      whereSelect,
      addressInput,
      fromInput,
      toInput,
      payeeInput,
      taxInput,
      feesInput,
      getCurrency,
    ),
    setDeclaredValue: (v: string) => {
      valueInput.value = v;
    },
    isFirstAuthorService: isFirstAuthor,
    isFirstMoney,
    get userModifiedValue() {
      return userModifiedValue;
    },
  };
}
