import { errText, field, text } from "../dom";
import { currentLanguage, t } from "../i18n/i18n";
import { textLangSelect } from "../i18n/text_lang";
import { currencyMinorExponent, CURRENCY_CODES, minorToDecimal, MoneyInputError, toMinorUnits } from "../money";
import { call } from "../rpc";
import { attachSizeLine } from "../screens/quote_size";
import { buildDealTerms, type CommitmentRowFields } from "../screens/quote_terms";
import { dealTotals, updateDealTotalsElements } from "../deals/deal_totals";
import { fillQuoteForm, type FormValues, type ListingLike, type SavedDefaultsLike } from "./defaults";
import { createCommitmentRow, type RowHandles } from "./rows";

const formRowHandles = new WeakMap<HTMLElement, RowHandles[]>();

export type QuoteFormError =
  | { key: string; args?: Record<string, string | number> }
  | { raw: string };

export type QuoteFormResult =
  | { terms: Record<string, unknown> }
  | { error: QuoteFormError };

export function formatQuoteError(err: QuoteFormError): string {
  if ("raw" in err) return err.raw;
  switch (err.key) {
    case "quote-scope-required":
      return t("quote-scope-required");
    case "quote-currency-required":
      return t("quote-currency-required");
    case "quote-currency-unknown":
      return t("quote-currency-unknown", err.args);
    case "quote-amount-required":
      return t("quote-amount-required");
    case "quote-payee-required":
      return t("quote-payee-required");
    case "quote-value-required":
      return t("deal-error-value-required");
    default:
      return err.key;
  }
}

export function readQuoteForm(form: HTMLElement): QuoteFormResult {
  const scopeInput = form.querySelector<HTMLInputElement>(".quote-scope-input");
  const scope = scopeInput?.value.trim() ?? "";
  if (!scope) {
    return { error: { key: "quote-scope-required" } };
  }

  const currencyInput = form.querySelector<HTMLInputElement>(".quote-currency-input");
  const currency = (currencyInput?.value.trim() || "INR").toUpperCase();
  if (!currency) {
    return { error: { key: "quote-currency-required" } };
  }
  if (!CURRENCY_CODES.has(currency)) {
    return { error: { key: "quote-currency-unknown", args: { currency } } };
  }

  const timingSelect = form.querySelector<HTMLSelectElement>(".quote-timing-select");
  const timing = (timingSelect?.value || "after-work") as "before-work" | "after-work";

  const cancInput = form.querySelector<HTMLInputElement>(".quote-canc-input");
  const refInput = form.querySelector<HTMLInputElement>(".quote-ref-input");
  const dispInput = form.querySelector<HTMLInputElement>(".quote-disp-input");

  const rowEls = form.querySelectorAll<HTMLElement>(".quote-commitment-row");
  const rowHandles: RowHandles[] = formRowHandles.get(form) || [];

  const rawRows: CommitmentRowFields[] = [];

  for (let i = 0; i < rowEls.length; i++) {
    const handle = rowHandles[i];
    let rowData: CommitmentRowFields | undefined;
    if (handle) {
      rowData = handle.read();
    } else {
      const rowEl = rowEls[i];
      const giver = (rowEl.querySelector<HTMLSelectElement>(".quote-row-giver-select")?.value || "quote-author") as "quote-author" | "counterparty";
      const kind = (rowEl.querySelector<HTMLSelectElement>(".quote-row-kind-select")?.value || "service") as "service" | "goods" | "money";
      const desc = rowEl.querySelector<HTMLInputElement>(".quote-row-desc-input")?.value.trim() || "";
      const valStr = rowEl.querySelector<HTMLInputElement>(".quote-row-value-input")?.value.trim() || "";
      const exp = currencyMinorExponent(currency) ?? 2;
      let declaredValueMinor = 0;
      let rowError: string | undefined;
      if (valStr) {
        try {
          declaredValueMinor = toMinorUnits(valStr, exp) ?? 0;
        } catch (e) {
          rowError = e instanceof MoneyInputError ? e.message : String(e);
        }
      }
      if (rowError) {
        return { error: { raw: rowError } };
      }
      rowData = { giver, kind, description: desc, declaredValueMinor };
    }

    if (!rowData) continue;

    if (rowData.error) {
      return { error: { raw: rowData.error } };
    }

    rawRows.push(rowData);
  }

  const firstMoney = rawRows.find((r) => r.kind === "money");
  const rows: CommitmentRowFields[] = [];

  for (let i = 0; i < rawRows.length; i++) {
    const rowData = rawRows[i];

    if (i === 0 && rowData.giver === "quote-author" && rowData.kind === "service") {
      if (!rowData.description) {
        rowData.description = scope;
      }
      if (rowData.declaredValueMinor <= 0 && firstMoney && firstMoney.declaredValueMinor > 0) {
        rowData.declaredValueMinor = firstMoney.declaredValueMinor;
      }
    }

    if (rowData.kind === "money") {
      if (rowData.declaredValueMinor <= 0) {
        return { error: { key: "quote-amount-required" } };
      }
      if (!rowData.payee && i === 1) {
        const payeeInput = form.querySelector<HTMLInputElement>(".quote-payee-input");
        rowData.payee = payeeInput?.value.trim() || "";
      }
      if (!rowData.payee) {
        return { error: { key: "quote-payee-required" } };
      }
    } else {
      if (rowData.declaredValueMinor <= 0) {
        return { error: { key: "quote-value-required" } };
      }
    }

    rows.push(rowData);
  }

  if (rows.length === 0) {
    return { error: { key: "quote-amount-required" } };
  }

  const langSelect = form.querySelector<HTMLSelectElement>(".quote-text-lang");
  const lang = langSelect?.value || currentLanguage();

  const terms = buildDealTerms(
    {
      scope,
      currency,
      timing,
      cancellation: cancInput?.value,
      refund: refInput?.value,
      dispute: dispInput?.value,
      rows,
    },
    lang,
  );

  return { terms };
}

function buildMoreTermsSection(
  initialValues: FormValues,
  quoteLang: HTMLSelectElement,
): {
  moreTerms: HTMLDetailsElement;
  timingSelect: HTMLSelectElement;
  cancInput: HTMLInputElement;
  refInput: HTMLInputElement;
  dispInput: HTMLInputElement;
  hoursInput: HTMLInputElement;
} {
  const moreTerms = document.createElement("details");
  moreTerms.className = "quote-more-terms";
  const moreSummary = document.createElement("summary");
  moreSummary.textContent = t("deal-more-terms");
  moreTerms.appendChild(moreSummary);

  const timingSelect = document.createElement("select");
  timingSelect.className = "quote-timing-select";
  const optAfter = document.createElement("option");
  optAfter.value = "after-work";
  optAfter.textContent = t("quote-timing-after");
  const optBefore = document.createElement("option");
  optBefore.value = "before-work";
  optBefore.textContent = t("quote-timing-before");
  timingSelect.append(optAfter, optBefore);
  timingSelect.value = initialValues.timing;

  const cancInput = document.createElement("input");
  cancInput.placeholder = t("quote-cancellation-placeholder");
  cancInput.defaultValue = initialValues.cancellation;
  cancInput.className = "quote-canc-input";

  const refInput = document.createElement("input");
  refInput.placeholder = t("quote-refund-placeholder");
  refInput.defaultValue = initialValues.refund;
  refInput.className = "quote-ref-input";

  const dispInput = document.createElement("input");
  dispInput.placeholder = t("quote-dispute-placeholder");
  dispInput.defaultValue = initialValues.dispute;
  dispInput.className = "quote-disp-input";

  const hoursInput = document.createElement("input");
  hoursInput.placeholder = t("quote-hours-placeholder");
  hoursInput.defaultValue = String(Math.round(initialValues.quoteValidSecs / 3600));
  hoursInput.type = "number";
  hoursInput.className = "quote-hours-input";

  const saveDefaultsBtn = document.createElement("button");
  saveDefaultsBtn.type = "button";
  saveDefaultsBtn.className = "button quote-save-defaults-button";
  saveDefaultsBtn.textContent = t("deal-save-defaults");
  const saveStatus = document.createElement("span");
  saveStatus.className = "quote-save-defaults-status";

  saveDefaultsBtn.onclick = async () => {
    saveStatus.textContent = "";
    const lang = quoteLang.value || currentLanguage();
    const hours = Number.parseInt(hoursInput.value || "24", 10);
    const quote_valid_secs = Math.max(300, hours * 3600);
    try {
      await call("deal.defaults-set", {
        cancellation_terms: { lang, text: cancInput.value.trim() },
        refund_terms: { lang, text: refInput.value.trim() },
        dispute_path: { lang, text: dispInput.value.trim() },
        quote_valid_secs,
      });
      saveStatus.textContent = t("deal-saved-defaults");
    } catch (err) {
      saveStatus.textContent = errText(err);
    }
  };

  moreTerms.append(
    timingSelect,
    cancInput,
    refInput,
    dispInput,
    hoursInput,
    saveDefaultsBtn,
    saveStatus,
  );

  return { moreTerms, timingSelect, cancInput, refInput, dispInput, hoursInput };
}

async function submitQuote(
  form: HTMLElement,
  reqCard: { record_id?: string; card_type?: string; data?: Record<string, unknown> },
  hoursInput: HTMLInputElement,
  submitBtn: HTMLButtonElement,
  errP: HTMLElement,
  detachSizeLine: () => void,
  refresh: () => Promise<void>,
) {
  errP.textContent = "";
  const parsed = readQuoteForm(form);
  if ("error" in parsed) {
    errP.textContent = formatQuoteError(parsed.error);
    return;
  }
  const terms = parsed.terms;

  const hours = Number.parseInt(hoursInput.value || "24", 10);
  const expires_in_secs = Math.max(300, hours * 3600);

  submitBtn.disabled = true;
  try {
    let enquiryRecordId = reqCard.record_id ?? "";
    let quoteId: string | undefined;
    if ((reqCard as Record<string, unknown>).card_type === "quote") {
      const data = reqCard.data as Record<string, unknown> | undefined;
      quoteId = data?.quote_id as string | undefined;
      const answers = data?.answers as { kind?: string; record_id?: string } | undefined;
      if (answers?.kind === "enquiry" && answers.record_id) {
        enquiryRecordId = answers.record_id;
      } else if (data?.request_record_id) {
        enquiryRecordId = data.request_record_id as string;
      }
    }

    await call("quote.set", {
      enquiry_record_id: enquiryRecordId,
      ...(quoteId ? { quote_id: quoteId } : {}),
      expires_in_secs,
      terms,
    });
    detachSizeLine();
    form.remove();
    await refresh();
  } catch (err) {
    errP.textContent = t("quote-failed", { detail: errText(err) });
    submitBtn.disabled = false;
  }
}

async function populateFromListingAndDefaults(
  reqCard: { data?: Record<string, unknown> },
  builtIn: { authorDid: string; cancellation: string; refund: string; dispute: string },
  elements: {
    currencyInput: () => HTMLInputElement | null;
    cancInput: HTMLInputElement;
    refInput: HTMLInputElement;
    dispInput: HTMLInputElement;
    hoursInput: HTMLInputElement;
    rowHandles: RowHandles[];
    onRowChange: () => void;
  },
) {
  const listingId = (reqCard.data as Record<string, unknown> | undefined)?.listing_id as string | undefined;

  let listingPayload: ListingLike | null = null;
  if (listingId) {
    try {
      const getRes = await call<{ listing?: ListingLike; envelope?: string }>("listing.get", { listing_id: listingId });
      if (getRes && "listing" in getRes && getRes.listing) {
        listingPayload = getRes.listing;
      } else if (getRes?.envelope) {
        const verifyRes = await call<{ verified: boolean; payload?: Record<string, unknown> }>("listing.verify", {
          envelope: getRes.envelope,
        });
        if (verifyRes?.verified && verifyRes.payload) {
          listingPayload = verifyRes.payload as unknown as ListingLike;
        }
      }
    } catch {
      // ignore
    }
  }

  let savedDefaults: SavedDefaultsLike | null = null;
  try {
    savedDefaults = (await call("deal.defaults-get")) as SavedDefaultsLike | null;
  } catch {
    // ignore
  }

  if (!listingPayload && !savedDefaults) return;

  const filled = fillQuoteForm(listingPayload, savedDefaults, builtIn);

  const currInput = elements.currencyInput();
  if (currInput && (currInput.value.trim() === "" || currInput.value === "INR")) {
    currInput.value = filled.currency;
  }

  const firstMoney = elements.rowHandles.find((h) => h.isFirstMoney);
  if (firstMoney) {
    const amountInput = firstMoney.element.querySelector<HTMLInputElement>(".quote-amount-input");
    if (amountInput && amountInput.value.trim() === "" && filled.amountMinor !== undefined) {
      const exp = currencyMinorExponent(filled.currency) ?? 2;
      amountInput.value = minorToDecimal(filled.amountMinor, exp);
    }
    const payeeInput = firstMoney.element.querySelector<HTMLInputElement>(".quote-payee-input");
    if (payeeInput && (payeeInput.value.trim() === "" || payeeInput.value === builtIn.authorDid) && filled.payee) {
      payeeInput.value = filled.payee;
    }
  }

  const firstAuthor = elements.rowHandles.find((h) => h.isFirstAuthorService);
  if (firstAuthor) {
    const whereSelect = firstAuthor.element.querySelector<HTMLSelectElement>(".quote-where-select");
    if (whereSelect && filled.where) {
      whereSelect.value = filled.where;
    }
    const addressInput = firstAuthor.element.querySelector<HTMLInputElement>(".quote-address-input");
    if (addressInput && addressInput.value.trim() === "" && filled.address) {
      addressInput.value = filled.address;
    }
  }

  if (elements.cancInput.value.trim() === "" || elements.cancInput.value === builtIn.cancellation) {
    elements.cancInput.value = filled.cancellation;
  }
  if (elements.refInput.value.trim() === "" || elements.refInput.value === builtIn.refund) {
    elements.refInput.value = filled.refund;
  }
  if (elements.dispInput.value.trim() === "" || elements.dispInput.value === builtIn.dispute) {
    elements.dispInput.value = filled.dispute;
  }
  if (elements.hoursInput.value.trim() === "" || elements.hoursInput.value === "24") {
    const hours = Math.min(168, Math.max(1, Math.round(filled.quoteValidSecs / 3600)));
    elements.hoursInput.value = String(hours);
  }

  elements.onRowChange();
}

export function openQuoteForm(
  anchor: HTMLElement,
  reqCard: { record_id?: string; card_type?: string; data?: Record<string, unknown> },
  convId: string,
  refresh: () => Promise<void>,
  preFill?: { terms?: Record<string, unknown> },
  myDid?: string,
): Promise<void> {
  const existing = anchor.querySelector(".quote-form");
  if (existing) {
    existing.remove();
    return Promise.resolve();
  }

  if (!preFill && reqCard.data?.terms) {
    preFill = { terms: reqCard.data.terms as Record<string, unknown> };
  }

  const form = document.createElement("div");
  form.className = "quote-form";

  const builtIn = {
    authorDid: myDid || "",
    cancellation: t("quote-default-cancellation"),
    refund: t("quote-default-refund"),
    dispute: t("quote-default-dispute"),
  };

  const initialValues = fillQuoteForm(null, null, builtIn);
  const quoteLang = textLangSelect("quote-text-lang", currentLanguage());

  const scopeInput = document.createElement("input");
  scopeInput.placeholder = t("quote-scope-placeholder");
  scopeInput.className = "quote-scope-input";
  if (preFill?.terms?.scope) {
    scopeInput.value =
      typeof preFill.terms.scope === "object" && preFill.terms.scope !== null && "text" in preFill.terms.scope
        ? String((preFill.terms.scope as { text: unknown }).text ?? "")
        : String(preFill.terms.scope);
  }

  const rowsContainer = document.createElement("div");
  rowsContainer.className = "quote-rows-container";

  const rowHandles: RowHandles[] = [];
  formRowHandles.set(form, rowHandles);

  const currency = initialValues.currency;

  const totalsBlock = document.createElement("div");
  totalsBlock.className = "quote-totals-block";
  const giveP = document.createElement("p");
  giveP.className = "quote-total-give";
  const receiveP = document.createElement("p");
  receiveP.className = "quote-total-receive";
  const diffP = document.createElement("p");
  diffP.className = "quote-total-diff";
  totalsBlock.append(giveP, receiveP, diffP);

  const updateTotals = () => {
    const commitments: Array<{ giver: string; declared_value_minor: number }> = [];
    for (const h of rowHandles) {
      const data = h.read();
      if (data && !data.error) {
        commitments.push({ giver: data.giver, declared_value_minor: data.declaredValueMinor });
      }
    }
    const curr = (form.querySelector<HTMLInputElement>(".quote-currency-input")?.value.trim() || currency || "INR").toUpperCase();
    const totals = dealTotals(commitments, "quote-author");
    updateDealTotalsElements({ giveEl: giveP, receiveEl: receiveP, diffEl: diffP }, totals, curr);
  };

  const syncAmountToService = () => {
    const firstAuthor = rowHandles.find((h) => h.isFirstAuthorService);
    const firstMoney = rowHandles.find((h) => h.isFirstMoney);
    if (firstAuthor && firstMoney && !firstAuthor.userModifiedValue) {
      const moneyData = firstMoney.read();
      if (moneyData && !moneyData.error) {
        const exp = currencyMinorExponent(currency) ?? 2;
        const valStr = minorToDecimal(moneyData.declaredValueMinor, exp);
        firstAuthor.setDeclaredValue(valStr);
      }
    }
  };

  const onRowChange = () => {
    syncAmountToService();
    updateTotals();
  };

  const addRowBtn = document.createElement("button");
  addRowBtn.type = "button";
  addRowBtn.className = "button quote-add-row-button deal-add-line";
  addRowBtn.textContent = t("deal-add-line");

  const renderRow = (initialData: Partial<CommitmentRowFields>, isFirstAuthor: boolean, isFirstMoney: boolean) => {
    const idx = rowHandles.length;
    const handle = createCommitmentRow(
      idx,
      initialData,
      isFirstAuthor,
      isFirstMoney,
      () => form.querySelector<HTMLInputElement>(".quote-currency-input")?.value.trim() || currency || "INR",
      onRowChange,
      () => {
        if (rowHandles.length <= 1) return;
        const removeIdx = rowHandles.indexOf(handle);
        if (removeIdx >= 0) {
          rowHandles.splice(removeIdx, 1);
          handle.element.remove();
          addRowBtn.disabled = rowHandles.length >= 16;
          onRowChange();
        }
      },
    );
    rowHandles.push(handle);
    rowsContainer.appendChild(handle.element);
    addRowBtn.disabled = rowHandles.length >= 16;
  };

  addRowBtn.onclick = () => {
    if (rowHandles.length >= 16) return;
    renderRow({ giver: "quote-author", kind: "service" }, false, false);
    onRowChange();
  };

  const preCommitments = preFill?.terms?.commitments as Array<Record<string, unknown>> | undefined;
  if (preCommitments && Array.isArray(preCommitments) && preCommitments.length > 0) {
    let seenAuthor = false;
    let seenMoney = false;
    for (const c of preCommitments) {
      const isAuthor = c.giver === "quote-author";
      const isMoney = c.kind === "currency" || c.resource_kind === "currency";
      const isFirstAuthor = isAuthor && !seenAuthor;
      const isFirstMoney = isMoney && !seenMoney;
      if (isFirstAuthor) seenAuthor = true;
      if (isFirstMoney) seenMoney = true;

      const rawDesc = c.description;
      const desc =
        typeof rawDesc === "object" && rawDesc !== null && "text" in rawDesc
          ? String((rawDesc as { text: unknown }).text ?? "")
          : String(rawDesc ?? "");

      const w = c.window as { earliest_secs?: number; latest_secs?: number } | undefined;
      const loc = c.location as { where?: import("../listings/editor").ServiceLocationKind; address?: string } | undefined;

      renderRow(
        {
          giver: isAuthor ? "quote-author" : "counterparty",
          kind: isMoney ? "money" : (c.resource_kind === "goods" ? "goods" : "service"),
          description: desc,
          declaredValueMinor: Number(c.declared_value_minor ?? 0),
          where: loc?.where,
          address: loc?.address,
          window:
            w?.earliest_secs !== undefined && w?.latest_secs !== undefined
              ? { earliest_secs: w.earliest_secs, latest_secs: w.latest_secs }
              : undefined,
          payee: typeof c.payee === "string" ? c.payee : undefined,
          taxMinor: Number(c.tax_minor ?? 0),
          feesMinor: Number(c.fees_minor ?? 0),
        },
        isFirstAuthor,
        isFirstMoney,
      );
    }
  } else {
    renderRow({ giver: "quote-author", kind: "service" }, true, false);
    renderRow({ giver: "counterparty", kind: "money", payee: initialValues.payee }, false, true);
  }

  const { moreTerms, cancInput, refInput, dispInput, hoursInput } = buildMoreTermsSection(initialValues, quoteLang);

  const submitBtn = text("button", t("quote-send-button"), "button submit-quote-button") as HTMLButtonElement;
  const cancelBtn = text("button", t("common-cancel"), "button cancel-quote-button") as HTMLButtonElement;
  const errP = text("p", "", "quote-form-error");

  const detachSizeLine = attachSizeLine(form, () => {
    const res = readQuoteForm(form);
    return "terms" in res ? res.terms : undefined;
  });

  cancelBtn.onclick = () => {
    detachSizeLine();
    form.remove();
  };

  submitBtn.onclick = () =>
    submitQuote(form, reqCard, hoursInput, submitBtn, errP, detachSizeLine, refresh);

  form.append(
    field(t("quote-text-language"), quoteLang),
    scopeInput,
    rowsContainer,
    addRowBtn,
    moreTerms,
    totalsBlock,
    submitBtn,
    cancelBtn,
    errP,
  );

  anchor.appendChild(form);
  updateTotals();

  return populateFromListingAndDefaults(reqCard, builtIn, {
    currencyInput: () => form.querySelector<HTMLInputElement>(".quote-currency-input"),
    cancInput,
    refInput,
    dispInput,
    hoursInput,
    rowHandles,
    onRowChange,
  });
}
