import { errText, refiller, text } from "../dom";
import { codeWord, currentLanguage, messageId, t } from "../i18n/i18n";
import { onFeed } from "../events";
import { call, RpcError } from "../rpc";
import { whoami } from "../session/login";
import { renderCard } from "../cards/render";
import { renderRefusedCard } from "../cards/refused";
import { toMinorUnits, currencyMinorExponent } from "../money";
import type { ServiceLocationKind } from "../listings/editor";
import { serviceLocationEditorLabel } from "../listings/labels";
import { type SearchMatch, searchHits } from "./message_search";
import { categoryPicker } from "../taxonomy/category_picker";

/// Message id for each message state, shown as a word. A `Map`, so a state
/// from another node can never hit an inherited property.
const STATE_LABELS = new Map<string, string>([
  ["pending", messageId("messages-state-pending")],
  ["deposited", messageId("messages-state-deposited")],
  ["delivered", messageId("messages-state-delivered")],
  ["failed", messageId("messages-state-failed")],
]);

/// The word for a message state; a state this table does not know is shown
/// as it came, never inferred.
export function messageStateWord(state: string): string {
  return codeWord(STATE_LABELS, state);
}

interface ConversationRow {
  id: string;
  kind?: "direct" | "group";
  peer_address: string;
  peer_person_did?: string;
  last_activity_ms: number;
  message_count: number;
}

export type MessageState = "pending" | "deposited" | "delivered" | "failed";

interface MessageRow {
  id: string;
  conversation: string;
  author: string;
  direction: "incoming" | "outgoing";
  sender_timestamp_ms: number;
  content_type: string;
  body_encoding: "utf8" | "base64";
  body?: string;
  state: MessageState;
  last_error?: string;
  refused?: string | null;
  restored?: boolean;
  deleted_at_secs?: number;
}

export interface CardRow {
  message_id: string;
  conversation: string;
  direction: "incoming" | "outgoing";
  sender_timestamp_ms: number;
  card_type: string;
  version: number;
  known: boolean;
  verified: boolean;
  expired: boolean;
  reason?: string;
  issuer?: string;
  record_id?: string;
  data?: {
    request_id?: string;
    request_record_id?: string;
    sequence?: number;
    counterparty_did?: string;
    terms?: {
      quote_expires_at_secs?: number;
    };
    quote_expires_at_secs?: number;
    [key: string]: unknown;
  };
  stored_at_secs: number;
  declined?: boolean;
  version_count?: number;
  agreement_payee?: string;
  agreement_payment_methods?: string[];
}

export async function renderMessages(container: HTMLElement, currentDid?: string) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "messages-screen";

  box.appendChild(text("h2", t("messages-title")));

  const myDid = currentDid || (await whoami())?.person_did || "";

  const layout = document.createElement("div");
  layout.className = "messages-layout";
  const listPane = document.createElement("div");
  listPane.className = "conversation-list";
  const threadPane = document.createElement("div");
  threadPane.className = "conversation-thread";
  layout.append(listPane, threadPane);
  box.appendChild(layout);

  // --- Start a new conversation -------------------------------------------
  const openRow = document.createElement("div");
  openRow.className = "open-conversation";
  const openInput = document.createElement("input");
  openInput.placeholder = t("messages-open-placeholder");
  openInput.className = "open-conversation-input";
  const openBtn = text("button", t("messages-open-button"), "button") as HTMLButtonElement;
  const openErr = text("p", "", "open-error");
  openBtn.onclick = async () => {
    openErr.textContent = "";
    const v = openInput.value.trim();
    if (!v) return;
    openBtn.disabled = true;
    try {
      const res = await call<{ conversation_id: string }>("conversation.open", { address: v });
      openInput.value = "";
      await reloadList(res.conversation_id);
    } catch (err) {
      openErr.textContent = t("messages-open-failed", { detail: errText(err) });
    }
    openBtn.disabled = false;
  };
  openRow.append(openInput, openBtn, openErr);
  listPane.appendChild(openRow);

  // --- Search -----------------------------------------------------------
  const searchRow = document.createElement("div");
  searchRow.className = "message-search";
  const searchInput = document.createElement("input");
  searchInput.placeholder = t("messages-search-placeholder");
  searchInput.className = "message-search-input";
  const searchBtn = text("button", t("messages-search-button"), "button") as HTMLButtonElement;
  const searchResults = document.createElement("div");
  searchResults.className = "search-results";
  const fillSearch = refiller(searchResults, async (into: DocumentFragment, q: string) => {
    if (!q) return;
    try {
      const res = await call<{ matches: SearchMatch[] }>("conversation.search", { query: q, kind: "direct" });
      const hits = searchHits(res.matches, q);
      if (hits.length === 0) {
        into.appendChild(text("p", t("messages-search-empty"), "search-empty"));
        return;
      }
      for (const h of hits) {
        const item = document.createElement("div");
        item.className = "search-hit";
        item.appendChild(text("div", h.snippet, "snippet"));
        item.onclick = () => reloadList(h.conversation);
        into.appendChild(item);
      }
    } catch (err) {
      into.appendChild(text("p", t("messages-search-failed", { detail: errText(err) })));
    }
  });
  searchBtn.onclick = () => void fillSearch(searchInput.value.trim());
  searchRow.append(searchInput, searchBtn);
  listPane.append(searchRow, searchResults);

  // --- The list ---------------------------------------------------------
  const listItems = document.createElement("div");
  listItems.className = "conversation-items";
  listPane.appendChild(listItems);

  let activeConvId: string | null = null;
  let activeFillHistory: (() => Promise<void>) | null = null;

  const onThreadActive = (convId: string, fill: () => Promise<void>) => {
    activeConvId = convId;
    activeFillHistory = fill;
  };

  // Resolves to the rows when this run drew the list, else `undefined` (the
  // load failed, or a newer run replaced this one).
  const fillList = refiller(listItems, async (into: DocumentFragment, select?: string) => {
    let rows: ConversationRow[] = [];
    try {
      const res = await call<{ conversations: ConversationRow[] }>("conversation.list", { kind: "direct" });
      rows = res.conversations;
    } catch (err) {
      into.appendChild(text("p", t("common-load-failed", { detail: errText(err) })));
      return undefined;
    }
    if (rows.length === 0) {
      into.appendChild(text("p", t("messages-conversations-empty"), "conversations-empty"));
      return rows;
    }
    const currentSelect = select ?? activeConvId ?? undefined;
    for (const r of rows) {
      const row = document.createElement("div");
      row.className = "conversation-item";
      if (r.id === currentSelect) row.classList.add("selected");
      row.appendChild(text("div", r.peer_person_did || r.peer_address, "peer-label"));
      row.appendChild(
        text("div", t("messages-count", { count: r.message_count }), "count-label"),
      );
      row.onclick = () => {
        listItems.querySelectorAll(".conversation-item").forEach((el) => el.classList.remove("selected"));
        row.classList.add("selected");
        renderThread(threadPane, r, myDid, onThreadActive);
      };
      into.appendChild(row);
    }
    return rows;
  });

  async function reloadList(select?: string) {
    if (select) activeConvId = select;
    const rows = await fillList(select ?? activeConvId ?? undefined);
    if (rows === undefined) {
      return;
    }
    if (rows.length === 0) {
      activeConvId = null;
      activeFillHistory = null;
      threadPane.replaceChildren();
      return;
    }
    const target = select
      ? rows.find((r) => r.id === select)
      : (activeConvId ? rows.find((r) => r.id === activeConvId) : rows[0]);
    if (target) await renderThread(threadPane, target, myDid, onThreadActive);
  }

  const removeFeed = onFeed(async (event) => {
    if (!box.isConnected) {
      removeFeed();
      return;
    }
    if ("reload" in event && event.reload) {
      if (activeFillHistory) {
        await activeFillHistory();
      }
      if (!activeConvId) {
        await reloadList();
      } else {
        await fillList(activeConvId);
      }
      return;
    }
    if ("kind" in event) {
      if (activeConvId && event.subject === activeConvId && activeFillHistory) {
        await activeFillHistory();
      }
      const hasConv =
        event.kind.startsWith("conversation.") ||
        ("kinds" in event && Boolean(event.kinds?.some((k) => k.startsWith("conversation."))));
      if (hasConv) {
        if (!activeConvId) {
          await reloadList();
        } else {
          await fillList(activeConvId);
        }
      }
    }
  });

  await reloadList();
  container.appendChild(box);
}

async function renderThread(
  pane: HTMLElement,
  conv: ConversationRow,
  myDid: string,
  onActive?: (convId: string, fill: () => Promise<void>) => void,
) {
  pane.replaceChildren();
  pane.appendChild(text("h3", conv.peer_person_did || conv.peer_address));

  const messagesHost = document.createElement("div");
  messagesHost.className = "thread-messages";
  pane.appendChild(messagesHost);

  // --- Compose message row --------------------------------------------------
  const composeRow = document.createElement("div");
  composeRow.className = "compose-row";
  const composeInput = document.createElement("input");
  composeInput.placeholder = t("messages-compose-placeholder");
  composeInput.className = "compose-input";
  const sendBtn = text("button", t("messages-send-button"), "button") as HTMLButtonElement;
  const composeErr = text("p", "", "compose-error");
  sendBtn.onclick = async () => {
    composeErr.textContent = "";
    const body = composeInput.value;
    if (!body.trim()) return;
    sendBtn.disabled = true;
    try {
      await call("conversation.send", { conversation: conv.id, body });
      composeInput.value = "";
      await fillHistory();
    } catch (err) {
      composeErr.textContent = t("messages-send-failed", { detail: errText(err) });
    }
    sendBtn.disabled = false;
  };
  composeRow.append(composeInput, sendBtn, composeErr);
  pane.appendChild(composeRow);

  // --- Send a request section -----------------------------------------------
  const requestSection = document.createElement("div");
  requestSection.className = "send-request-section";

  const toggleReqBtn = text("button", t("request-toggle-button"), "button toggle-request-form") as HTMLButtonElement;
  requestSection.appendChild(toggleReqBtn);

  const reqForm = document.createElement("div");
  reqForm.className = "request-form";
  reqForm.style.display = "none";

  const descInput = document.createElement("textarea");
  descInput.placeholder = t("request-description-placeholder");
  descInput.className = "request-desc-input";

  const catsPicker = categoryPicker({
    className: "request-cats-input",
    max: 8,
  });

  const listingInput = document.createElement("input");
  listingInput.placeholder = t("request-listing-placeholder");
  listingInput.className = "request-listing-input";

  // The person signs the text they read: the same string is drawn here and
  // sent with the request.
  const dataUseNotice = t("request-data-use-notice");
  const noticeP = text("p", dataUseNotice, "data-use-notice");

  const submitReqBtn = text("button", t("request-submit-button"), "button send-request-button") as HTMLButtonElement;
  const reqErr = text("p", "", "request-form-error");

  toggleReqBtn.onclick = () => {
    reqForm.style.display = reqForm.style.display === "none" ? "block" : "none";
  };

  submitReqBtn.onclick = async () => {
    const desc = descInput.value.trim();
    if (!desc) {
      reqErr.textContent = t("request-description-required");
      return;
    }
    submitReqBtn.disabled = true;
    reqErr.textContent = "";
    try {
      const categories = catsPicker.codes();
      await call("request.set", {
        conversation: conv.id,
        description: desc,
        lang: currentLanguage(),
        categories,
        listing_id: listingInput.value.trim() || undefined,
        data_use_notice: dataUseNotice,
      });
      descInput.value = "";
      catsPicker.clear();
      listingInput.value = "";
      reqForm.style.display = "none";
      await fillHistory();
    } catch (err) {
      reqErr.textContent = t("request-send-failed", { detail: errText(err) });
    } finally {
      submitReqBtn.disabled = false;
    }
  };

  reqForm.append(descInput, catsPicker.root, listingInput, noticeP, submitReqBtn, reqErr);
  requestSection.appendChild(reqForm);
  pane.appendChild(requestSection);

  const fillHistory = refiller(messagesHost, async (into: DocumentFragment) => {
    const cardMap = new Map<string, CardRow>();
    let allCards: CardRow[] = [];
    try {
      await call("transaction.sync", { conversation: conv.id });
      const threadRes = await call<{ cards: CardRow[] }>("transaction.thread", {
        conversation: conv.id,
      });
      if (threadRes?.cards) {
        allCards = threadRes.cards;
        for (const card of allCards) {
          cardMap.set(card.message_id, card);
        }
      }
    } catch {
      // transaction service may be unavailable
    }

    const newestRequestMap = computeNewestRequestMap(allCards);

    let messages: MessageRow[] = [];
    try {
      const res = await call<{ messages: MessageRow[] }>("conversation.history", {
        conversation: conv.id,
      });
      messages = res.messages;
    } catch (err) {
      into.appendChild(text("p", t("messages-load-failed", { detail: errText(err) })));
      return;
    }

    for (const m of messages) {
      into.appendChild(
        messageElement(m, cardMap, allCards, newestRequestMap, conv.id, myDid, fillHistory),
      );
    }
  });

  onActive?.(conv.id, fillHistory);
  await fillHistory();
}

export function computeNewestRequestMap(allCards: CardRow[]): Map<string, string> {
  const newestRequestMap = new Map<string, string>();
  for (const c of allCards) {
    if (c.card_type === "request" && c.verified && c.data?.request_id && c.record_id) {
      const reqId = c.data.request_id;
      const currNewestRecord = newestRequestMap.get(reqId);
      if (!currNewestRecord) {
        newestRequestMap.set(reqId, c.record_id);
      } else {
        const existing = allCards.find((x) => x.record_id === currNewestRecord);
        const cVer = c.version_count ?? 0;
        const exVer = existing?.version_count ?? 0;
        if (
          cVer > exVer ||
          (cVer === exVer && c.sender_timestamp_ms > (existing?.sender_timestamp_ms ?? 0))
        ) {
          newestRequestMap.set(reqId, c.record_id);
        }
      }
    }
  }
  return newestRequestMap;
}

export function isQuoteSuperseded(
  cardRow: CardRow,
  allCards: CardRow[],
  newestRequestMap: Map<string, string>,
): boolean {
  const reqRecordId = cardRow.data?.request_record_id;
  if (cardRow.card_type === "quote" && cardRow.verified && reqRecordId) {
    const matchingReq = allCards.find(
      (c) =>
        c.card_type === "request" &&
        (c.record_id === reqRecordId || c.data?.request_id === cardRow.data?.request_id),
    );
    if (matchingReq && matchingReq.data?.request_id) {
      const newestRecordId = newestRequestMap.get(matchingReq.data.request_id);
      if (newestRecordId && newestRecordId !== reqRecordId) {
        return true;
      }
    }
  }
  return false;
}

/** What a sender is told when the other side turned a delivered message away. */
/** A short explanation for a state word that needs one, else null. */
export function stateHint(state: MessageState): string | null {
  return state === "deposited"
    ? t("messages-state-hint-deposited")
    : null;
}

/** Only a failed message of the user's own history can be sent again. */
export function canRetry(m: { state: MessageState; deleted_at_secs?: number; restored?: boolean }): boolean {
  return m.state === "failed" && m.deleted_at_secs == null && !m.restored;
}

export function refusalText(reason: string): string {
  if (reason === "rate-limited") {
    return t("messages-refused-rate-limited");
  }
  return t("messages-refused-other", { reason });
}

function messageElement(
  m: MessageRow,
  cardMap: Map<string, CardRow>,
  allCards: CardRow[],
  newestRequestMap: Map<string, string>,
  convId: string,
  myDid: string,
  refresh: () => Promise<void>,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = `message message-${m.direction}`;
  wrap.dataset.state = m.state;

  if (m.content_type === "application/vnd.roym.card+json") {
    const cardRow = cardMap.get(m.id);
    if (cardRow) {
      if (cardRow.known && cardRow.verified) {
        wrap.appendChild(
          renderCard({
            type: cardRow.card_type,
            version: cardRow.version,
            data: cardRow.data,
            agreement_payee: cardRow.agreement_payee,
            agreement_payment_methods: cardRow.agreement_payment_methods,
          }),
        );
      } else if (cardRow.known && !cardRow.verified) {
        wrap.appendChild(renderRefusedCard(cardRow.card_type, cardRow.version, cardRow.reason));
      } else {
        wrap.appendChild(renderCard({ type: cardRow.card_type, version: cardRow.version, data: {} }));
      }

      // Check if quote answers an earlier version of the request
      if (isQuoteSuperseded(cardRow, allCards, newestRequestMap)) {
        const supP = document.createElement("p");
        supP.className = "quote-superseded-notice";
        supP.textContent = t("quote-superseded-notice");
        wrap.appendChild(supP);
      }

      // Render contextual card action buttons
      renderCardActions(wrap, cardRow, convId, myDid, refresh);
    } else {
      wrap.appendChild(renderRefusedCard("card", 1, t("messages-unfiled-card")));
    }
  } else {
    if (m.deleted_at_secs != null) {
      wrap.appendChild(text("span", t("messages-message-deleted"), "message-body deleted"));
    } else {
      wrap.appendChild(text("span", m.body ?? t("messages-no-body"), "message-body"));
    }
  }

  // The state word is exactly what the API returned -- never inferred, and
  // "delivered" is never shown until the service says so.
  const stateSpan = text("span", messageStateWord(m.state), `message-state state-${m.state}`);
  const hint = stateHint(m.state);
  if (hint) {
    stateSpan.title = hint;
  }
  wrap.appendChild(stateSpan);
  if (m.refused) {
    wrap.appendChild(text("span", refusalText(m.refused), "message-refused"));
  }
  if (m.state === "failed" && m.last_error) {
    wrap.appendChild(text("span", m.last_error, "message-error"));
  }

  const actions = document.createElement("div");
  actions.className = "message-actions";

  // A deleted or restored message is never offered for sending again; the
  // service refuses both.
  if (canRetry(m)) {
    const retryBtn = text("button", t("messages-retry-button"), "button retry-message") as HTMLButtonElement;
    retryBtn.onclick = async () => {
      retryBtn.disabled = true;
      try {
        await call("conversation.retry", { message_id: m.id });
        await refresh();
      } catch {
        retryBtn.disabled = false;
      }
    };
    actions.appendChild(retryBtn);
  }

  if (m.deleted_at_secs == null) {
    const deleteBtn = text("button", t("messages-delete-button"), "button delete-message") as HTMLButtonElement;
    deleteBtn.onclick = () => openDeleteDialog(wrap, m, refresh);
    actions.appendChild(deleteBtn);
  }

  wrap.appendChild(actions);
  return wrap;
}

export function renderCardActions(
  wrap: HTMLElement,
  card: CardRow,
  convId: string,
  myDid: string,
  refresh: () => Promise<void>,
) {
  // Quote this request: on verified request not issued by me
  if (card.card_type === "request" && card.verified) {
    const notIssuedByMe = card.direction === "incoming" || (myDid && card.issuer !== myDid);
    if (notIssuedByMe) {
      const quoteBtn = text("button", t("quote-request-button"), "button quote-request-button") as HTMLButtonElement;
      quoteBtn.onclick = () => openQuoteForm(wrap, card, convId, refresh);
      wrap.appendChild(quoteBtn);
    }
  }

  // Accept / Decline: on verified quote for me, not expired, not declined
  if (card.card_type === "quote" && card.verified) {
    const forMe = !!myDid && card.data?.counterparty_did === myDid;
    const nowSecs = Math.floor(Date.now() / 1000);
    const expSecs = card.data?.terms?.quote_expires_at_secs ?? card.data?.quote_expires_at_secs;
    const isExpired = card.expired || (expSecs !== undefined && nowSecs > expSecs);
    const isDeclined = card.declined === true;

    if (forMe && !isExpired && !isDeclined) {
      const actionsDiv = document.createElement("div");
      actionsDiv.className = "quote-actions";

      const acceptBtn = text("button", t("messages-accept-button"), "button accept-quote-button") as HTMLButtonElement;
      const acceptErr = text("p", "", "accept-quote-error");

      acceptBtn.onclick = async () => {
        acceptBtn.disabled = true;
        acceptErr.textContent = "";
        try {
          await call("agreement.accept", { quote_record_id: card.record_id });
          await refresh();
        } catch (err) {
          acceptBtn.disabled = false;
          const msg = err instanceof RpcError ? err.message : String(err);
          if (msg.includes("quote-expired") || msg.includes("expired")) {
            acceptErr.textContent = t("messages-quote-expired");
          } else {
            acceptErr.textContent = t("messages-accept-failed", { detail: errText(err) });
          }
        }
      };

      const declineBtn = text("button", t("messages-decline-button"), "button decline-quote-button") as HTMLButtonElement;
      declineBtn.onclick = () => openDeclineDialog(wrap, card, refresh);

      actionsDiv.append(acceptBtn, declineBtn, acceptErr);
      wrap.appendChild(actionsDiv);
    }
  }
}

export function openDeclineDialog(anchor: HTMLElement, card: CardRow, refresh: () => Promise<void>) {
  const existing = anchor.querySelector(".decline-dialog");
  if (existing) existing.remove();

  const dialog = document.createElement("div");
  dialog.className = "decline-dialog";
  dialog.appendChild(text("p", t("messages-decline-note"), "decline-note"));

  const noteInput = document.createElement("input");
  noteInput.placeholder = t("messages-decline-note-placeholder");
  noteInput.className = "decline-note-input";

  const confirmBtn = text("button", t("messages-decline-confirm"), "button confirm-decline") as HTMLButtonElement;
  const cancelBtn = text("button", t("common-cancel"), "button cancel-decline") as HTMLButtonElement;
  const err = text("p", "", "decline-error");

  cancelBtn.onclick = () => dialog.remove();
  confirmBtn.onclick = async () => {
    confirmBtn.disabled = true;
    try {
      await call("quote.decline", {
        quote_record_id: card.record_id,
        note: noteInput.value.trim() || undefined,
      });
      await refresh();
    } catch (e) {
      err.textContent = t("messages-decline-failed", { detail: errText(e) });
      confirmBtn.disabled = false;
    }
  };

  dialog.append(noteInput, confirmBtn, cancelBtn, err);
  anchor.appendChild(dialog);
}

export function openQuoteForm(
  anchor: HTMLElement,
  reqCard: CardRow,
  convId: string,
  refresh: () => Promise<void>,
) {
  const existing = anchor.querySelector(".quote-form");
  if (existing) {
    existing.remove();
    return;
  }

  const form = document.createElement("div");
  form.className = "quote-form";

  const scopeInput = document.createElement("input");
  scopeInput.placeholder = t("quote-scope-placeholder");
  scopeInput.className = "quote-scope-input";

  const currencyInput = document.createElement("input");
  currencyInput.placeholder = t("quote-currency-placeholder");
  currencyInput.defaultValue = "EUR";
  currencyInput.className = "quote-currency-input";

  const amountInput = document.createElement("input");
  amountInput.placeholder = t("quote-amount-placeholder");
  amountInput.className = "quote-amount-input";

  const taxInput = document.createElement("input");
  taxInput.placeholder = t("quote-tax-placeholder");
  taxInput.className = "quote-tax-input";

  const feesInput = document.createElement("input");
  feesInput.placeholder = t("quote-fees-placeholder");
  feesInput.className = "quote-fees-input";

  const payeeInput = document.createElement("input");
  payeeInput.placeholder = t("quote-payee-placeholder");
  payeeInput.className = "quote-payee-input";

  const timingSelect = document.createElement("select");
  timingSelect.className = "quote-timing-select";
  const optAfter = document.createElement("option");
  optAfter.value = "after-work";
  optAfter.textContent = t("quote-timing-after");
  const optBefore = document.createElement("option");
  optBefore.value = "before-work";
  optBefore.textContent = t("quote-timing-before");
  timingSelect.append(optAfter, optBefore);

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

  const addressWrap = document.createElement("div");
  addressWrap.className = "quote-address-wrap";
  const addressNotice = text("p", t("quote-address-disclosure-notice"), "address-disclosure-notice");
  const addressInput = document.createElement("input");
  addressInput.placeholder = t("quote-address-placeholder");
  addressInput.className = "quote-address-input";
  addressWrap.append(addressNotice, addressInput);

  whereSelect.onchange = () => {
    addressWrap.style.display = whereSelect.value === "at-receiver" ? "block" : "none";
  };

  const cancInput = document.createElement("input");
  cancInput.placeholder = t("quote-cancellation-placeholder");
  cancInput.defaultValue = t("quote-default-cancellation");
  cancInput.className = "quote-canc-input";

  const refInput = document.createElement("input");
  refInput.placeholder = t("quote-refund-placeholder");
  refInput.defaultValue = t("quote-default-refund");
  refInput.className = "quote-ref-input";

  const dispInput = document.createElement("input");
  dispInput.placeholder = t("quote-dispute-placeholder");
  dispInput.defaultValue = t("quote-default-dispute");
  dispInput.className = "quote-disp-input";

  const hoursInput = document.createElement("input");
  hoursInput.placeholder = t("quote-hours-placeholder");
  hoursInput.defaultValue = "24";
  hoursInput.type = "number";
  hoursInput.className = "quote-hours-input";

  const submitBtn = text("button", t("quote-send-button"), "button submit-quote-button") as HTMLButtonElement;
  const cancelBtn = text("button", t("common-cancel"), "button cancel-quote-button") as HTMLButtonElement;
  const errP = text("p", "", "quote-form-error");

  cancelBtn.onclick = () => form.remove();

  submitBtn.onclick = async () => {
    errP.textContent = "";
    const scope = scopeInput.value.trim();
    if (!scope) {
      errP.textContent = t("quote-scope-required");
      return;
    }
    const currency = currencyInput.value.trim().toUpperCase();
    if (!currency) {
      errP.textContent = t("quote-currency-required");
      return;
    }
    const exp = currencyMinorExponent(currency);
    if (exp === undefined) {
      errP.textContent = t("quote-currency-unknown", { currency });
      return;
    }
    let amountMinor: number | undefined;
    try {
      amountMinor = toMinorUnits(amountInput.value, exp);
    } catch (e) {
      errP.textContent = errText(e);
      return;
    }
    if (amountMinor === undefined) {
      errP.textContent = t("quote-amount-required");
      return;
    }

    let taxMinor = 0;
    if (taxInput.value.trim()) {
      try {
        taxMinor = toMinorUnits(taxInput.value, exp) ?? 0;
      } catch (e) {
        errP.textContent = errText(e);
        return;
      }
    }

    let feesMinor = 0;
    if (feesInput.value.trim()) {
      try {
        feesMinor = toMinorUnits(feesInput.value, exp) ?? 0;
      } catch (e) {
        errP.textContent = errText(e);
        return;
      }
    }

    const payee = payeeInput.value.trim();
    if (!payee) {
      errP.textContent = t("quote-payee-required");
      return;
    }

    const hours = Number.parseInt(hoursInput.value || "24", 10);
    const expires_in_secs = Math.max(300, hours * 3600);

    const where = whereSelect.value as ServiceLocationKind;
    const address = where === "at-receiver" ? addressInput.value.trim() : undefined;

    submitBtn.disabled = true;
    try {
      await call("quote.set", {
        request_record_id: reqCard.record_id,
        expires_in_secs,
        terms: {
          lang: currentLanguage(),
          scope,
          currency,
          amount_minor: amountMinor,
          tax_minor: taxMinor,
          fees_minor: feesMinor,
          payee,
          payment_timing: timingSelect.value,
          location: {
            where,
            address,
          },
          cancellation_terms: cancInput.value.trim(),
          refund_terms: refInput.value.trim(),
          dispute_path: dispInput.value.trim(),
        },
      });
      form.remove();
      await refresh();
    } catch (err) {
      errP.textContent = t("quote-failed", { detail: errText(err) });
      submitBtn.disabled = false;
    }
  };

  form.append(
    scopeInput,
    currencyInput,
    amountInput,
    taxInput,
    feesInput,
    payeeInput,
    timingSelect,
    whereSelect,
    addressWrap,
    cancInput,
    refInput,
    dispInput,
    hoursInput,
    submitBtn,
    cancelBtn,
    errP,
  );
  anchor.appendChild(form);
}

function openDeleteDialog(anchor: HTMLElement, m: MessageRow, refresh: () => Promise<void>) {
  const existing = anchor.querySelector(".delete-dialog");
  if (existing) existing.remove();

  const dialog = document.createElement("div");
  dialog.className = "delete-dialog";
  const isSent = m.direction === "outgoing";
  dialog.appendChild(text("p", isSent ? t("messages-delete-note-sent") : t("messages-delete-note-received"), "delete-note"));

  let askPeer = isSent;
  if (isSent) {
    const label = document.createElement("label");
    label.className = "ask-peer";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = true;
    cb.onchange = () => {
      askPeer = cb.checked;
    };
    label.append(cb, text("span", t("messages-delete-ask-peer")));
    dialog.appendChild(label);
  }

  const confirmBtn = text("button", t("messages-delete-confirm"), "button confirm-delete") as HTMLButtonElement;
  const cancelBtn = text("button", t("common-cancel"), "button cancel-delete") as HTMLButtonElement;
  const err = text("p", "", "delete-error");
  cancelBtn.onclick = () => dialog.remove();
  confirmBtn.onclick = async () => {
    confirmBtn.disabled = true;
    try {
      await call("conversation.delete-message", { message_id: m.id, ask_peer: askPeer });
      await refresh();
    } catch (e) {
      err.textContent = t("messages-delete-failed", { detail: errText(e) });
      confirmBtn.disabled = false;
    }
  };
  dialog.append(confirmBtn, cancelBtn, err);
  anchor.appendChild(dialog);
}
