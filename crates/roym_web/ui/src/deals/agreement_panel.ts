import { errText } from "../dom";
import { codeWord, currentLanguage, t } from "../i18n/i18n";
import { readerLanguages } from "../i18n/text_lang";
import { bestVersion } from "../records/record_text";
import { formatMinor } from "../money";
import { call } from "../rpc";
import {
  COMMITMENT_REASON_WORDS,
  COMMITMENT_STATE_WORDS,
} from "../cards/templates/agreement_progress";
import { closeList, type AgreementViewForClose } from "./close_list";
import { dealTotals, renderDealTotalsBlock, type Commitment as TotalCommitment } from "./deal_totals";
import type { Role } from "./version_summary";

export interface AgreementViewCommitment {
  commitment_id: string;
  giver: string;
  kind?: string;
  resource_kind?: string;
  declared_value_minor?: number | bigint | string;
  description?: { lang?: string; text?: string } | string;
  window?: { earliest_secs: number; latest_secs: number };
  location?: { where: string; address?: string };
  payee?: string;
  tax_minor?: number | bigint;
  fees_minor?: number | bigint;
  [key: string]: unknown;
}

export interface AgreementViewProgress {
  commitment_id: string;
  state: string;
  reason?: string;
  claimed?: boolean;
  halves?: string[];
  [key: string]: unknown;
}

export interface AgreementView {
  quote_record_id: string;
  quote_id?: string;
  conversation?: string;
  counterparty_did?: string;
  quote_author_did?: string;
  state?: "agreed" | "closed" | "cancelled" | "not-formed" | string;
  terms: {
    currency?: string;
    commitments: AgreementViewCommitment[];
    scope?: { lang?: string; text?: string } | string;
    [key: string]: unknown;
  };
  commitments?: AgreementViewProgress[];
  [key: string]: unknown;
}

interface SettleResult {
  record_id?: string;
  role?: string;
  message_id?: string;
  state?: string;
}

function commitmentDescription(c: AgreementViewCommitment, currency: string): string {
  if (typeof c.description === "object" && c.description !== null) {
    const desc = c.description as { lang?: string; text?: string; translations?: Array<{ lang: string; text: string }> };
    if (desc.translations && desc.translations.length > 0) {
      const versions = [
        { lang: desc.lang || "en", fields: desc.text || "", byAuthor: true },
        ...desc.translations.map((tr) => ({ lang: tr.lang, fields: tr.text, byAuthor: true })),
      ];
      const idx = bestVersion(versions, readerLanguages());
      const chosen = versions[idx]?.fields;
      if (chosen) return chosen;
    }
    if (desc.text) return desc.text;
  }
  if (typeof c.description === "string" && c.description) {
    return c.description;
  }
  if (c.kind === "currency" && c.declared_value_minor !== undefined) {
    return formatMinor(BigInt(c.declared_value_minor), currency);
  }
  return "";
}

function renderCommitmentRow(
  c: AgreementViewCommitment,
  view: AgreementView,
  myRole: Role,
  currency: string,
  isAgreed: boolean,
  ownHalves: Map<string, string> | undefined,
  refresh: () => Promise<void>,
  onReload: () => Promise<void>,
): HTMLElement {
  const row = document.createElement("div");
  row.className = `agreement-commitment-row agreement-commitment-${c.commitment_id}`;

  const isViewerGiver = c.giver === myRole;
  const desc = commitmentDescription(c, currency);
  const p = view.commitments?.find((item) => item.commitment_id === c.commitment_id);
  const state = p?.state ?? "promised";
  const isFinal = ["fulfilled", "acknowledged", "ended-unconfirmed", "failed", "cancelled"].includes(state);

  const giverSpan = document.createElement("span");
  giverSpan.className = "agreement-commitment-giver";
  giverSpan.textContent = isViewerGiver ? t("deal-giver-me") : t("deal-giver-them");

  const descSpan = document.createElement("span");
  descSpan.className = "agreement-commitment-desc";
  descSpan.textContent = desc;

  const valSpan = document.createElement("span");
  valSpan.className = "agreement-commitment-value";
  valSpan.textContent =
    c.declared_value_minor !== undefined ? formatMinor(BigInt(c.declared_value_minor), currency) : "";

  const stateSpan = document.createElement("span");
  stateSpan.className = `agreement-commitment-state state-${state}`;
  if (state === "ended-unconfirmed") {
    stateSpan.textContent =
      p?.claimed === true ? t("deal-unconfirmed-claimed") : t("deal-unconfirmed-unrecorded");
  } else {
    const stateWord = codeWord(COMMITMENT_STATE_WORDS, state);
    const reasonWord =
      (state === "failed" || state === "cancelled") && p?.reason
        ? ` (${codeWord(COMMITMENT_REASON_WORDS, p.reason)})`
        : "";
    stateSpan.textContent = `${stateWord}${reasonWord}`;
  }

  row.append(giverSpan, descSpan, valSpan, stateSpan);

  const actionBox = document.createElement("div");
  actionBox.className = "agreement-commitment-action";

  const ownHalfId = ownHalves?.get(c.commitment_id);
  const ownHalfInProgress = ownHalfId && p?.halves?.includes(ownHalfId);
  const isWaiting = myRole === "counterparty" && Boolean(ownHalfId && !ownHalfInProgress);

  if (isAgreed && !isFinal) {
    if (isWaiting) {
      const waitingSpan = document.createElement("span");
      waitingSpan.className = "deal-waiting-other-node";
      waitingSpan.textContent = t("deal-waiting-other-node");
      actionBox.appendChild(waitingSpan);
    } else if (isViewerGiver && (p?.claimed === true || state === "claimed")) {
      const claimedSpan = document.createElement("span");
      claimedSpan.className = "deal-claimed-giver";
      claimedSpan.textContent = t("deal-claimed-giver");
      actionBox.appendChild(claimedSpan);
    } else {
      const actionBtn = buildActionBtn(c, isViewerGiver, view, row, onReload, refresh);
      actionBox.appendChild(actionBtn);
    }
  }

  row.appendChild(actionBox);
  return row;
}

function buildActionBtn(
  c: AgreementViewCommitment,
  isViewerGiver: boolean,
  view: AgreementView,
  row: HTMLElement,
  onReload: () => Promise<void>,
  refresh: () => Promise<void>,
): HTMLButtonElement {
  const isMoney = c.kind === "currency" || c.resource_kind === "currency";
  const actionBtn = document.createElement("button");
  actionBtn.type = "button";

  if (isViewerGiver) {
    if (isMoney) {
      actionBtn.className = "button deal-action-paid-button";
      actionBtn.textContent = t("deal-action-paid");
    } else {
      actionBtn.className = "button deal-action-delivered-button";
      actionBtn.textContent = t("deal-action-delivered");
    }
  } else {
    actionBtn.className = "button deal-action-received-button";
    actionBtn.textContent = t("deal-action-received");
  }

  actionBtn.onclick = () => {
    const existingDialog = row.querySelector(".settle-dialog");
    if (existingDialog) {
      existingDialog.remove();
      return;
    }
    const dialog = renderSettleDialog(c, isMoney, view.quote_record_id, onReload, refresh);
    row.appendChild(dialog);
  };

  return actionBtn;
}

function renderSettleDialog(
  c: AgreementViewCommitment,
  isMoney: boolean,
  quoteRecordId: string,
  onReload: () => Promise<void>,
  refresh: () => Promise<void>,
): HTMLElement {
  const dialog = document.createElement("div");
  dialog.className = "settle-dialog";

  const noteInput = document.createElement("input");
  noteInput.className = "settle-note-input settlement_note";
  noteInput.placeholder = t("deal-settle-note-placeholder");

  const confirmBtn = document.createElement("button");
  confirmBtn.type = "button";
  confirmBtn.className = "button settle-confirm-button";
  confirmBtn.textContent = t("deal-settle-confirm");

  const cancelBtn = document.createElement("button");
  cancelBtn.type = "button";
  cancelBtn.className = "button settle-cancel-button";
  cancelBtn.textContent = t("common-cancel");

  const errP = document.createElement("p");
  errP.className = "settle-error error-text";

  cancelBtn.onclick = () => dialog.remove();

  confirmBtn.onclick = async () => {
    confirmBtn.disabled = true;
    errP.textContent = "";
    const noteText = noteInput.value.trim();
    const settlement_note = noteText ? { lang: currentLanguage(), text: noteText } : undefined;

    try {
      let res: SettleResult | null = null;
      if (isMoney) {
        res = await call<SettleResult>("payment.acknowledge", {
          agreement: quoteRecordId,
          commitment_id: c.commitment_id,
          ...(settlement_note ? { settlement_note } : {}),
        });
      } else {
        res = await call<SettleResult>("fulfilment.sign", {
          agreement: quoteRecordId,
          commitment_id: c.commitment_id,
          ...(settlement_note ? { settlement_note } : {}),
        });
      }

      if (res?.state === "failed" && res?.message_id) {
        const msgId = res.message_id;
        errP.textContent = t("error-network", { detail: res.state });
        const retryBtn = document.createElement("button");
        retryBtn.type = "button";
        retryBtn.className = "button retry-message";
        retryBtn.textContent = t("messages-retry-button");
        retryBtn.onclick = async () => {
          retryBtn.disabled = true;
          try {
            await call("conversation.retry", { message_id: msgId });
            dialog.remove();
            await onReload();
            await refresh();
          } catch (e) {
            retryBtn.disabled = false;
            errP.textContent = errText(e);
          }
        };
        dialog.appendChild(retryBtn);
        return;
      }

      dialog.remove();
      await onReload();
      await refresh();
    } catch (err) {
      confirmBtn.disabled = false;
      errP.textContent = errText(err);
    }
  };

  dialog.append(noteInput, confirmBtn, cancelBtn, errP);
  return dialog;
}

function openCloseListDialog(
  container: HTMLElement,
  viewForClose: AgreementViewForClose,
  asSide: "giver" | "receiver",
  currency: string,
  quoteRecordId: string,
  onReload: () => Promise<void>,
  refresh: () => Promise<void>,
) {
  const existing = container.querySelector(".close-list");
  if (existing) {
    existing.remove();
    return;
  }

  const items = closeList(viewForClose, asSide, Math.floor(Date.now() / 1000));
  const closeListDiv = document.createElement("div");
  closeListDiv.className = "close-list";

  const itemsContainer = document.createElement("div");
  itemsContainer.className = "close-list-items";

  for (const item of items) {
    const itemRow = document.createElement("div");
    itemRow.className = `close-list-item close-list-item-${item.commitment_id}`;

    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.className = "close-list-item-checkbox";
    cb.dataset.commitmentId = item.commitment_id;
    cb.checked = item.checked;

    const descSpan = document.createElement("span");
    descSpan.className = "close-list-item-desc";
    descSpan.textContent = commitmentDescription(item.commitment as unknown as AgreementViewCommitment, currency);

    const valSpan = document.createElement("span");
    valSpan.className = "close-list-item-value";
    valSpan.textContent = formatMinor(item.value, currency);

    itemRow.append(cb, descSpan, valSpan);

    if (item.notStarted) {
      const notStartedSpan = document.createElement("span");
      notStartedSpan.className = "close-list-not-started";
      notStartedSpan.textContent = t("deal-close-not-started");
      itemRow.appendChild(notStartedSpan);
    }

    itemsContainer.appendChild(itemRow);
  }

  const signBtn = document.createElement("button");
  signBtn.type = "button";
  signBtn.className = "button deal-close-sign-button";
  signBtn.textContent = t("deal-close-sign-button");

  const cancelBtn = document.createElement("button");
  cancelBtn.type = "button";
  cancelBtn.className = "button deal-close-cancel-button";
  cancelBtn.textContent = t("common-cancel");

  const errP = document.createElement("p");
  errP.className = "close-list-error error-text";

  cancelBtn.onclick = () => closeListDiv.remove();

  signBtn.onclick = async () => {
    const checkedBoxes = [...closeListDiv.querySelectorAll<HTMLInputElement>(".close-list-item-checkbox:checked")];
    const checkedIds = checkedBoxes.map((box) => box.dataset.commitmentId ?? "").filter(Boolean);
    if (checkedIds.length === 0) return;

    signBtn.disabled = true;
    errP.textContent = "";
    try {
      await call("agreement.close", {
        agreement: quoteRecordId,
        as: asSide,
        commitments: checkedIds,
      });
      closeListDiv.remove();
      await onReload();
      await refresh();
    } catch (err) {
      signBtn.disabled = false;
      errP.textContent = errText(err);
    }
  };

  closeListDiv.append(itemsContainer, signBtn, cancelBtn, errP);
  container.appendChild(closeListDiv);
}

function renderClosingActions(
  container: HTMLElement,
  view: AgreementView,
  myRole: Role,
  currency: string,
  onReload: () => Promise<void>,
  refresh: () => Promise<void>,
): HTMLElement | null {
  const closingActions = document.createElement("div");
  closingActions.className = "agreement-closing-actions";

  const nowSecs = Math.floor(Date.now() / 1000);
  const viewForClose = {
    ...view,
    my_role: myRole,
    role: myRole,
  } as unknown as AgreementViewForClose;

  const giveItems = closeList(viewForClose, "giver", nowSecs);
  const receiveItems = closeList(viewForClose, "receiver", nowSecs);

  if (giveItems.length > 0) {
    const giveAllBtn = document.createElement("button");
    giveAllBtn.type = "button";
    giveAllBtn.className = "button deal-close-give-button";
    giveAllBtn.textContent = t("deal-close-give-all");
    giveAllBtn.onclick = () =>
      openCloseListDialog(container, viewForClose, "giver", currency, view.quote_record_id, onReload, refresh);
    closingActions.appendChild(giveAllBtn);
  }

  if (receiveItems.length > 0) {
    const receiveAllBtn = document.createElement("button");
    receiveAllBtn.type = "button";
    receiveAllBtn.className = "button deal-close-receive-button";
    receiveAllBtn.textContent = t("deal-close-receive-all");
    receiveAllBtn.onclick = () =>
      openCloseListDialog(container, viewForClose, "receiver", currency, view.quote_record_id, onReload, refresh);
    closingActions.appendChild(receiveAllBtn);
  }

  return closingActions.children.length > 0 ? closingActions : null;
}

export function populateAgreementPanelView(
  container: HTMLElement,
  view: AgreementView,
  myDid: string,
  refresh: () => Promise<void>,
  onReload: () => Promise<void>,
  ownHalves?: Map<string, string>,
): void {
  container.className = `agreement-panel agreement-panel-${view.quote_record_id}`;
  const scopeText =
    typeof view.terms.scope === "object" && view.terms.scope !== null && "text" in view.terms.scope
      ? String((view.terms.scope as { text?: unknown }).text ?? "")
      : String(view.terms.scope ?? "");
  if (scopeText) {
    container.dataset.scope = scopeText;
  }

  container.replaceChildren();

  const myRole: Role =
    myDid && view.quote_author_did && myDid === view.quote_author_did ? "quote-author" : "counterparty";
  const currency = view.terms.currency ?? "INR";
  const isAgreed = view.state === "agreed";

  const list = document.createElement("div");
  list.className = "agreement-commitments-list";

  for (const c of view.terms.commitments) {
    list.appendChild(
      renderCommitmentRow(c, view, myRole, currency, isAgreed, ownHalves, refresh, onReload),
    );
  }
  container.appendChild(list);

  const totals = dealTotals(view.terms.commitments as unknown as TotalCommitment[], myRole);
  container.appendChild(renderDealTotalsBlock(totals, currency));

  if (isAgreed) {
    const closingActions = renderClosingActions(container, view, myRole, currency, onReload, refresh);
    if (closingActions) {
      container.appendChild(closingActions);
    }
  }
}

export function renderAgreementPanelView(
  view: AgreementView,
  myDid: string,
  refresh: () => Promise<void>,
  onReload: () => Promise<void>,
  ownHalves?: Map<string, string>,
): HTMLElement {
  const container = document.createElement("div");
  populateAgreementPanelView(container, view, myDid, refresh, onReload, ownHalves);
  return container;
}

async function fetchOwnHalves(view: AgreementView): Promise<Map<string, string>> {
  const ownHalves = new Map<string, string>();
  if (view.state !== "agreed") return ownHalves;

  const promises = view.terms.commitments.map(async (c) => {
    const isMoney = c.kind === "currency" || c.resource_kind === "currency";
    try {
      if (isMoney) {
        const res = await call<{ counterparty?: Array<{ record_id: string }> }>("payment.get", {
          agreement: view.quote_record_id,
          commitment_id: c.commitment_id,
        });
        const recId = res?.counterparty?.[res.counterparty.length - 1]?.record_id;
        if (recId) ownHalves.set(c.commitment_id, recId);
      } else {
        const res = await call<{ counterparty?: { record_id: string } }>("fulfilment.get", {
          agreement: view.quote_record_id,
          commitment_id: c.commitment_id,
        });
        const recId = res?.counterparty?.record_id;
        if (recId) ownHalves.set(c.commitment_id, recId);
      }
    } catch {
      // ignore get errors
    }
  });

  await Promise.all(promises);
  return ownHalves;
}

function renderAgreementLoadError(err: unknown, retry: () => Promise<void>): HTMLElement {
  const box = document.createElement("div");
  box.className = "agreement-load-error";

  const msg = document.createElement("p");
  msg.className = "error-text";
  msg.textContent = t("agreement-load-failed", { detail: errText(err) });

  const retryBtn = document.createElement("button");
  retryBtn.type = "button";
  retryBtn.className = "button agreement-retry-button";
  retryBtn.textContent = t("messages-retry-button");
  retryBtn.onclick = () => void retry();

  box.append(msg, retryBtn);
  return box;
}

export function renderAgreementPanel(
  quoteRecordId: string,
  myDid: string,
  refresh: () => Promise<void>,
): HTMLElement {
  const container = document.createElement("div");
  container.className = `agreement-panel agreement-panel-${quoteRecordId}`;

  const loadAndRender = async () => {
    try {
      const res = await call<AgreementView | null>("agreement.get", { quote_record_id: quoteRecordId });
      if (!res) return;

      const myRole =
        myDid && res.quote_author_did && myDid === res.quote_author_did ? "quote-author" : "counterparty";
      let ownHalves: Map<string, string> | undefined;
      if (myRole === "counterparty") {
        ownHalves = await fetchOwnHalves(res);
      }

      populateAgreementPanelView(container, res, myDid, refresh, loadAndRender, ownHalves);
    } catch (err) {
      container.replaceChildren(renderAgreementLoadError(err, loadAndRender));
    }
  };

  void loadAndRender();
  return container;
}
