import type { ServiceLocationKind } from "../listings/editor";
import { serviceLocationEditorLabel } from "../listings/labels";
import { t } from "../i18n/i18n";
import { currencyMinorExponent, toMinorUnits } from "../money";
import type { CommitmentRowFields } from "../screens/quote_terms";

export interface RowHandles {
  element: HTMLElement;
  read: () => CommitmentRowFields | undefined;
  setDeclaredValue: (val: string) => void;
  isFirstAuthorService: boolean;
  isFirstMoney: boolean;
  userModifiedValue: boolean;
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

  // Giver selector
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

  // Kind selector
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

  // Description input (not used for money)
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

  // Declared value input
  const valueWrap = document.createElement("div");
  valueWrap.className = "quote-row-value-wrap";
  const valueInput = document.createElement("input");
  valueInput.className = "quote-row-value-input";
  if (isFirstMoney) {
    valueInput.classList.add("quote-amount-input");
    valueInput.placeholder = t("quote-amount-placeholder");
  } else {
    valueInput.placeholder = t("quote-amount-placeholder");
  }
  if (initial.declaredValueMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    valueInput.value = String(initial.declaredValueMinor / Math.pow(10, exp));
  }
  valueInput.oninput = () => {
    userModifiedValue = true;
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

  // Location selector (for author service row)
  const locationWrap = document.createElement("div");
  locationWrap.className = "quote-row-location-wrap";
  const whereSelect = document.createElement("select");
  whereSelect.className = "quote-where-select";
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

  // Time window (for service/goods)
  const windowWrap = document.createElement("div");
  windowWrap.className = "quote-row-window-wrap";
  windowWrap.style.display = "none";
  const whenLabel = document.createElement("label");
  whenLabel.className = "quote-row-when-label";
  whenLabel.textContent = t("deal-row-when");
  const fromInput = document.createElement("input");
  fromInput.className = "quote-row-from-input";
  fromInput.type = "number";
  fromInput.placeholder = t("deal-from-secs-placeholder");
  const toInput = document.createElement("input");
  toInput.className = "quote-row-to-input";
  toInput.type = "number";
  toInput.placeholder = t("deal-to-secs-placeholder");
  if (initial.window) {
    fromInput.value = String(initial.window.earliest_secs);
    toInput.value = String(initial.window.latest_secs);
  }
  fromInput.oninput = onChanged;
  toInput.oninput = onChanged;
  windowWrap.append(whenLabel, fromInput, toInput);

  // Money fields (payee, tax, fees, and currency for first money row)
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
  payeeInput.className = "quote-payee-input";
  payeeInput.placeholder = t("quote-payee-placeholder");
  if (initial.payee) payeeInput.value = initial.payee;
  payeeInput.oninput = onChanged;

  const taxInput = document.createElement("input");
  taxInput.className = "quote-tax-input";
  taxInput.placeholder = t("quote-tax-placeholder");
  if (initial.taxMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    taxInput.value = String(initial.taxMinor / Math.pow(10, exp));
  }
  taxInput.oninput = onChanged;

  const feesInput = document.createElement("input");
  feesInput.className = "quote-fees-input";
  feesInput.placeholder = t("quote-fees-placeholder");
  if (initial.feesMinor !== undefined) {
    const exp = currencyMinorExponent(getCurrency()) ?? 2;
    feesInput.value = String(initial.feesMinor / Math.pow(10, exp));
  }
  feesInput.oninput = onChanged;

  moneyWrap.append(payeeInput, taxInput, feesInput);

  // Remove button
  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.className = "button quote-row-remove-button";
  removeBtn.textContent = t("deal-remove-line");
  removeBtn.onclick = onRemove;

  // Visibility toggle based on kind and giver
  const updateVisibility = () => {
    const kind = kindSelect.value;
    const giver = giverSelect.value;

    if (kind === "money") {
      descInput.style.display = "none";
      valueHint.style.display = "none";
      locationWrap.style.display = "none";
      windowWrap.style.display = "none";
      moneyWrap.style.display = "block";
    } else {
      descInput.style.display = "block";
      valueHint.style.display = "block";
      updateValueHint();
      moneyWrap.style.display = "none";
      windowWrap.style.display = "flex";

      if (kind === "service" && giver === "quote-author") {
        locationWrap.style.display = "block";
      } else {
        locationWrap.style.display = "none";
      }
    }
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

  row.append(
    giverSelect,
    kindSelect,
    descInput,
    valueWrap,
    locationWrap,
    windowWrap,
    moneyWrap,
    removeBtn,
  );

  const read = (): CommitmentRowFields | undefined => {
    const giver = giverSelect.value as "quote-author" | "counterparty";
    const kind = kindSelect.value as "service" | "goods" | "money";
    const exp = currencyMinorExponent(getCurrency()) ?? 2;

    const valStr = valueInput.value.trim();
    let declaredValueMinor = 0;
    if (valStr) {
      try {
        declaredValueMinor = toMinorUnits(valStr, exp) ?? 0;
      } catch {
        declaredValueMinor = Math.round(Number.parseFloat(valStr) * Math.pow(10, exp)) || 0;
      }
    }

    const res: CommitmentRowFields = {
      giver,
      kind,
      description: descInput.value.trim(),
      declaredValueMinor,
    };

    if (kind === "service" && giver === "quote-author") {
      res.where = whereSelect.value as ServiceLocationKind;
      if (addressInput.value.trim()) res.address = addressInput.value.trim();
    }

    if (fromInput.value || toInput.value) {
      const from = Number.parseInt(fromInput.value || "0", 10);
      const to = Number.parseInt(toInput.value || "0", 10);
      if (from > 0 || to > 0) {
        res.window = { earliest_secs: from, latest_secs: to };
      }
    }

    if (kind === "money") {
      res.payee = payeeInput.value.trim();
      if (taxInput.value.trim()) {
        res.taxMinor = toMinorUnits(taxInput.value.trim(), exp) ?? 0;
      }
      if (feesInput.value.trim()) {
        res.feesMinor = toMinorUnits(feesInput.value.trim(), exp) ?? 0;
      }
    }

    return res;
  };

  return {
    element: row,
    read,
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
