import { errText, field, text } from "../dom";
import { currentLanguage, t } from "../i18n/i18n";
import { textLangSelect } from "../i18n/text_lang";
import { currencyMinorExponent, CURRENCY_CODES, formatMinor, toMinorUnits } from "../money";
import { call } from "../rpc";
import { attachSizeLine } from "../screens/quote_size";
import { buildDealTerms, type CommitmentRowFields } from "../screens/quote_terms";
import { dealTotals } from "../deals/deal_totals";
import { fillQuoteForm } from "./defaults";
import { createCommitmentRow, type RowHandles } from "./rows";

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
  const rowHandles: RowHandles[] = (form as unknown as { _rowHandles?: RowHandles[] })._rowHandles || [];

  const rows: CommitmentRowFields[] = [];

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
      const declaredValueMinor = valStr ? (toMinorUnits(valStr, exp) ?? 0) : 0;
      rowData = { giver, kind, description: desc, declaredValueMinor };
    }

    if (!rowData) continue;

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

export function openQuoteForm(
  anchor: HTMLElement,
  reqCard: { record_id?: string; card_type?: string; data?: Record<string, unknown> },
  convId: string,
  refresh: () => Promise<void>,
  preFill?: { terms?: Record<string, unknown> },
) {
  const existing = anchor.querySelector(".quote-form");
  if (existing) {
    existing.remove();
    return;
  }

  if (!preFill && reqCard.data?.terms) {
    preFill = { terms: reqCard.data.terms as Record<string, unknown> };
  }

  const form = document.createElement("div");
  form.className = "quote-form";

  const builtIn = {
    authorDid: "",
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
  (form as unknown as { _rowHandles: RowHandles[] })._rowHandles = rowHandles;

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
      if (data) {
        commitments.push({ giver: data.giver, declared_value_minor: data.declaredValueMinor });
      }
    }
    const curr = (form.querySelector<HTMLInputElement>(".quote-currency-input")?.value.trim() || currency || "INR").toUpperCase();
    const totals = dealTotals(commitments, "quote-author");

    giveP.textContent = t("deal-you-give", { amount: formatMinor(totals.give, curr) });
    receiveP.textContent = t("deal-you-receive", { amount: formatMinor(totals.receive, curr) });

    if (totals.difference > 0n) {
      diffP.style.display = "block";
      diffP.textContent = t("deal-diff-you-receive-more", { amount: formatMinor(totals.difference, curr) });
    } else if (totals.difference < 0n) {
      diffP.style.display = "block";
      diffP.textContent = t("deal-diff-you-give-more", { amount: formatMinor(-totals.difference, curr) });
    } else {
      diffP.style.display = "none";
      diffP.textContent = "";
    }
  };

  const syncAmountToService = () => {
    const firstAuthor = rowHandles.find((h) => h.isFirstAuthorService);
    const firstMoney = rowHandles.find((h) => h.isFirstMoney);
    if (firstAuthor && firstMoney && !firstAuthor.userModifiedValue) {
      const moneyData = firstMoney.read();
      if (moneyData) {
        const exp = currencyMinorExponent(currency) ?? 2;
        const valStr = (moneyData.declaredValueMinor / Math.pow(10, exp)).toFixed(exp);
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
  addRowBtn.className = "button quote-add-row-button";
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

  // If pre-filled commitments exist, populate them; otherwise render the fresh 2 rows
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
      const loc = c.location as { where?: import("../../listings/editor").ServiceLocationKind; address?: string } | undefined;

      renderRow(
        {
          giver: isAuthor ? "quote-author" : "counterparty",
          kind: isMoney ? "money" : (c.resource_kind === "goods" ? "goods" : "service"),
          description: desc,
          declaredValueMinor: Number(c.declared_value_minor ?? 0),
          where: loc?.where,
          address: loc?.address,
          windowFromSecs: w?.earliest_secs,
          windowToSecs: w?.latest_secs,
          payee: typeof c.payee === "string" ? c.payee : undefined,
          taxMinor: Number(c.tax_minor ?? 0),
          feesMinor: Number(c.fees_minor ?? 0),
        },
        isFirstAuthor,
        isFirstMoney,
      );
    }
  } else {
    // Fresh form: Row 0 author service, Row 1 counterparty money
    renderRow({ giver: "quote-author", kind: "service" }, true, false);
    renderRow({ giver: "counterparty", kind: "money", payee: initialValues.payee }, false, true);
  }

  // More terms collapsible section
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

  submitBtn.onclick = async () => {
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
      const enquiryRecordId = ((reqCard as Record<string, unknown>).card_type === "quote"
        ? (((reqCard.data as Record<string, unknown>)?.request_record_id as string) ?? reqCard.record_id)
        : reqCard.record_id) ?? "";
      const quoteId = (reqCard as Record<string, unknown>).card_type === "quote"
        ? ((reqCard.data as Record<string, unknown>)?.quote_id as string | undefined)
        : undefined;

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
  };

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

  // Async load defaults from node settings if available
  void (async () => {
    try {
      const saved = await call("deal.defaults-get") as {
        cancellation_terms?: { text: string };
        refund_terms?: { text: string };
        dispute_path?: { text: string };
        quote_valid_secs?: number;
      } | null;
      if (saved) {
        if (saved.cancellation_terms?.text) cancInput.value = saved.cancellation_terms.text;
        if (saved.refund_terms?.text) refInput.value = saved.refund_terms.text;
        if (saved.dispute_path?.text) dispInput.value = saved.dispute_path.text;
        if (saved.quote_valid_secs) hoursInput.value = String(Math.round(saved.quote_valid_secs / 3600));
      }
    } catch {
      // ignore
    }
  })();
}
