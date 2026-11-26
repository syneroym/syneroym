import { errText } from "../dom";
import { codeWord, currentLanguage, t } from "../i18n/i18n";
import { formatMinor } from "../money";
import { call } from "../rpc";
import {
  COMMITMENT_REASON_WORDS,
  COMMITMENT_STATE_WORDS,
} from "../cards/templates/agreement_progress";
import { closeList, type AgreementViewForClose } from "./close_list";
import { dealTotals, type Commitment as TotalCommitment } from "./deal_totals";
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

function commitmentDescription(c: AgreementViewCommitment, currency: string): string {
  if (typeof c.description === "object" && c.description !== null && "text" in c.description) {
    return c.description.text ?? "";
  }
  if (typeof c.description === "string" && c.description) {
    return c.description;
  }
  if (c.kind === "currency" && c.declared_value_minor !== undefined) {
    return formatMinor(BigInt(c.declared_value_minor), currency);
  }
  return "";
}

export function renderAgreementPanelView(
  view: AgreementView,
  myDid: string,
  refresh: () => Promise<void>,
  onReload: () => Promise<void>,
): HTMLElement {
  const container = document.createElement("div");
  container.className = `agreement-panel agreement-panel-${view.quote_record_id}`;

  const myRole: Role =
    myDid && view.quote_author_did && myDid === view.quote_author_did ? "quote-author" : "counterparty";
  const currency = view.terms.currency ?? "INR";
  const isAgreed = view.state === "agreed";

  // Commitments list
  const list = document.createElement("div");
  list.className = "agreement-commitments-list";

  for (const c of view.terms.commitments) {
    const row = document.createElement("div");
    row.className = `agreement-commitment-row agreement-commitment-${c.commitment_id}`;

    const isViewerGiver = c.giver === myRole;
    const desc = commitmentDescription(c, currency);
    const p = view.commitments?.find((item) => item.commitment_id === c.commitment_id);
    const state = p?.state ?? "promised";
    const isFinal = ["fulfilled", "acknowledged", "ended-unconfirmed", "failed", "cancelled"].includes(
      state,
    );

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

    // Per-commitment action container
    const actionBox = document.createElement("div");
    actionBox.className = "agreement-commitment-action";

    if (isAgreed && !isFinal) {
      if (myRole === "counterparty" && p?.halves && p.halves.length > 0) {
        // Counterparty waiting state
        const waitingSpan = document.createElement("span");
        waitingSpan.className = "deal-waiting-other-node";
        waitingSpan.textContent = t("deal-waiting-other-node");
        actionBox.appendChild(waitingSpan);
      } else if (isViewerGiver && (p?.claimed === true || state === "claimed")) {
        // Giver already claimed
        const claimedSpan = document.createElement("span");
        claimedSpan.className = "deal-claimed-giver";
        claimedSpan.textContent = t("deal-claimed-giver");
        actionBox.appendChild(claimedSpan);
      } else {
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
              if (isMoney) {
                await call("payment.acknowledge", {
                  agreement: view.quote_record_id,
                  commitment_id: c.commitment_id,
                  ...(settlement_note ? { settlement_note } : {}),
                });
              } else {
                await call("fulfilment.sign", {
                  agreement: view.quote_record_id,
                  commitment_id: c.commitment_id,
                  ...(settlement_note ? { settlement_note } : {}),
                });
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
          row.appendChild(dialog);
        };

        actionBox.appendChild(actionBtn);
      }
    }

    row.appendChild(actionBox);
    list.appendChild(row);
  }

  container.appendChild(list);

  // Totals block
  const totalsBlock = document.createElement("div");
  totalsBlock.className = "quote-totals-block";
  const totals = dealTotals(view.terms.commitments as unknown as TotalCommitment[], myRole);

  const giveP = document.createElement("p");
  giveP.className = "quote-total-give";
  giveP.textContent = t("deal-you-give", { amount: formatMinor(totals.give, currency) });

  const receiveP = document.createElement("p");
  receiveP.className = "quote-total-receive";
  receiveP.textContent = t("deal-you-receive", { amount: formatMinor(totals.receive, currency) });

  totalsBlock.append(giveP, receiveP);

  if (totals.difference > 0n) {
    const diffP = document.createElement("p");
    diffP.className = "quote-total-diff";
    diffP.textContent = t("deal-diff-you-receive-more", { amount: formatMinor(totals.difference, currency) });
    totalsBlock.append(diffP);
  } else if (totals.difference < 0n) {
    const diffP = document.createElement("p");
    diffP.className = "quote-total-diff";
    diffP.textContent = t("deal-diff-you-give-more", { amount: formatMinor(-totals.difference, currency) });
    totalsBlock.append(diffP);
  }

  container.appendChild(totalsBlock);

  // Closing actions (AG-46)
  if (isAgreed) {
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

    const openCloseList = (asSide: "giver" | "receiver") => {
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
        descSpan.textContent = commitmentDescription(
          item.commitment as unknown as AgreementViewCommitment,
          currency,
        );

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
        const checkedBoxes = [
          ...closeListDiv.querySelectorAll<HTMLInputElement>(".close-list-item-checkbox:checked"),
        ];
        const checkedIds = checkedBoxes.map((box) => box.dataset.commitmentId ?? "").filter(Boolean);
        if (checkedIds.length === 0) return;

        signBtn.disabled = true;
        errP.textContent = "";
        try {
          await call("agreement.close", {
            agreement: view.quote_record_id,
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
      closingActions.appendChild(closeListDiv);
    };

    if (giveItems.length > 0) {
      const giveAllBtn = document.createElement("button");
      giveAllBtn.type = "button";
      giveAllBtn.className = "button deal-close-give-button";
      giveAllBtn.textContent = t("deal-close-give-all");
      giveAllBtn.onclick = () => openCloseList("giver");
      closingActions.appendChild(giveAllBtn);
    }

    if (receiveItems.length > 0) {
      const receiveAllBtn = document.createElement("button");
      receiveAllBtn.type = "button";
      receiveAllBtn.className = "button deal-close-receive-button";
      receiveAllBtn.textContent = t("deal-close-receive-all");
      receiveAllBtn.onclick = () => openCloseList("receiver");
      closingActions.appendChild(receiveAllBtn);
    }

    if (closingActions.children.length > 0) {
      container.appendChild(closingActions);
    }
  }

  return container;
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
      const viewEl = renderAgreementPanelView(res, myDid, refresh, loadAndRender);
      container.replaceChildren(...viewEl.children);
    } catch {
      // ignore get errors
    }
  };

  void loadAndRender();
  return container;
}
