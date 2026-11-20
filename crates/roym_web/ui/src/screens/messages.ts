import { messageLang, messageText } from "../chat/message_text";
import { errText, field, refiller, text } from "../dom";
import { codeWord, currentLanguage, messageId, t } from "../i18n/i18n";
import { languageList } from "../i18n/language_list";
import { languageName, textLangSelect } from "../i18n/text_lang";
import { onFeed } from "../events";
import { call, RpcError } from "../rpc";
import { whoami } from "../session/login";
import { renderCard } from "../cards/render";
import { renderRefusedCard } from "../cards/refused";
import { toMinorUnits, currencyMinorExponent } from "../money";
import type { ServiceLocationKind } from "../listings/editor";
import { serviceLocationEditorLabel } from "../listings/labels";
import { type SearchMatch, searchHits } from "./message_search";
import { waitingRedraw } from "./thread_redraw";
import { categoryPicker } from "../taxonomy/category_picker";
import { buildDealTerms } from "./quote_terms";
import { attachSizeLine } from "./quote_size";
import {
  computeNewestQuoteMap,
  computeNewestRequestMap,
  isQuoteSuperseded,
  newestVersionMap,
} from "./thread_enquiry";

/// Things the person has started inside a thread. A redraw from the event
/// feed waits until they are gone, so it never throws away typed text. An
/// element inside the thread that holds the person's input belongs here.
const OPEN_WORK = ".quote-form, .decline-dialog, .delete-dialog, .accept-mt-box";

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
  languages?: string[];
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
    enquiry_id?: string;
    enquiry_record_id?: string;
    sequence?: number;
    counterparty_did?: string;
    terms?: {
      lang?: string;
      quote_expires_at_secs?: number;
    };
    quote_expires_at_secs?: number;
    [key: string]: unknown;
  };
  stored_at_secs: number;
  repeat?: boolean;
  declined?: boolean;
  version_count?: number;
  deal_state?: string;
  agreement_payee?: string;
  agreement_payment_methods?: string[];
}

export interface RequestPreFill {
  listingId?: string;
  categories?: string[];
}

export interface MessagesInitialState {
  conversationId?: string;
  preFill?: RequestPreFill;
}

let pendingOpen: MessagesInitialState | null = null;
let activeReloadMessages: ((convId: string, preFill?: RequestPreFill) => Promise<void>) | null = null;

export async function openConversation(
  address: string,
  preFill?: RequestPreFill,
  container?: HTMLElement,
): Promise<string> {
  const res = await call<{ conversation_id: string }>("conversation.open", { address });
  const convId = res.conversation_id;

  if (container) {
    await renderMessages(container, undefined, { conversationId: convId, preFill });
    return convId;
  }

  const activeScreen = document.querySelector<HTMLElement>(".messages-screen");
  if (activeScreen && activeScreen.isConnected && activeReloadMessages) {
    await activeReloadMessages(convId, preFill);
    return convId;
  }

  pendingOpen = { conversationId: convId, preFill };

  const msgTabBtn =
    document.querySelector<HTMLButtonElement>('.tab-button[data-tab="messages"]') ??
    Array.from(document.querySelectorAll<HTMLButtonElement>(".tab-button")).find(
      (btn) => btn.textContent?.trim() === t("tab-messages").trim(),
    );

  if (msgTabBtn) {
    msgTabBtn.click();
  } else {
    const fallback =
      document.querySelector<HTMLElement>(".tab-content") ??
      document.querySelector<HTMLElement>(".content-area") ??
      document.querySelector<HTMLElement>("#app");
    if (fallback) {
      await renderMessages(fallback, undefined, { conversationId: convId, preFill });
    }
  }

  return convId;
}

export async function renderMessages(
  container: HTMLElement,
  currentDid?: string,
  initial?: MessagesInitialState,
) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "messages-screen";

  const initialConvId = initial?.conversationId ?? pendingOpen?.conversationId ?? undefined;
  const initialPreFill = initial?.preFill ?? pendingOpen?.preFill ?? undefined;
  pendingOpen = null;

  activeReloadMessages = async (convId: string, preFill?: RequestPreFill) => {
    await reloadList(convId, preFill);
  };

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
        const snip = text("div", h.snippet, "snippet") as HTMLElement;
        snip.setAttribute("lang", messageLang(h.languages ?? []));
        item.appendChild(snip);
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

  async function reloadList(select?: string, preFill?: RequestPreFill) {
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
    if (target) {
      await renderThread(
        threadPane,
        target,
        myDid,
        onThreadActive,
        target.id === select ? preFill : undefined,
      );
    }
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

  await reloadList(initialConvId, initialPreFill);
  container.appendChild(box);
}

async function renderThread(
  pane: HTMLElement,
  conv: ConversationRow,
  myDid: string,
  onActive?: (convId: string, fill: () => Promise<void>) => void,
  preFill?: RequestPreFill,
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

  const langList = languageList(
    "messages",
    3,
    [currentLanguage()],
    {
      add: t("messages-languages-add"),
      moveUp: t("messages-languages-move-up"),
      remove: t("messages-languages-remove"),
    },
    t("messages-languages-label"),
  );

  sendBtn.onclick = async () => {
    composeErr.textContent = "";
    const body = composeInput.value;
    if (!body.trim()) return;
    sendBtn.disabled = true;
    try {
      await call("conversation.send", {
        conversation: conv.id,
        body,
        languages: langList.value(),
      });
      composeInput.value = "";
      await redraw.now();
    } catch (err) {
      composeErr.textContent = t("messages-send-failed", { detail: errText(err) });
    }
    sendBtn.disabled = false;
  };
  const langField = document.createElement("div");
  langField.className = "field";
  langField.append(text("span", t("messages-languages-label")), langList.root);
  composeRow.append(composeInput, sendBtn, composeErr, langField);
  pane.appendChild(composeRow);

  const fillThread = async (into: DocumentFragment) => {
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
    const newestQuoteMap = computeNewestQuoteMap(allCards);

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
      if (m.content_type === "application/vnd.roym.card+json" && cardMap.get(m.id)?.repeat) {
        continue;
      }
      into.appendChild(
        messageElement(m, cardMap, allCards, newestRequestMap, newestQuoteMap, conv.id, myDid, redraw.now),
      );
    }
  };

  const redraw = waitingRedraw(messagesHost, fillThread, OPEN_WORK);
  renderRequestSection(pane, conv, redraw.now, preFill);
  onActive?.(conv.id, redraw.later);
  await redraw.now();
}

function renderRequestSection(
  pane: HTMLElement,
  conv: ConversationRow,
  redrawNow: () => Promise<void>,
  preFill?: RequestPreFill,
) {
  const requestSection = document.createElement("div");
  requestSection.className = "send-request-section";

  const toggleReqBtn = text("button", t("request-toggle-button"), "button toggle-request-form") as HTMLButtonElement;
  requestSection.appendChild(toggleReqBtn);

  const reqForm = document.createElement("div");
  reqForm.className = "request-form";
  reqForm.style.display = preFill ? "block" : "none";

  const descInput = document.createElement("textarea");
  descInput.placeholder = t("request-description-placeholder");
  descInput.className = "request-desc-input";

  const catsPicker = categoryPicker({
    className: "request-cats-input",
    max: 8,
    selected: preFill?.categories,
  });

  const reqLang = textLangSelect("request-text-lang", currentLanguage());

  const listingInput = document.createElement("input");
  listingInput.placeholder = t("request-listing-placeholder");
  listingInput.className = "request-listing-input";
  if (preFill?.listingId) {
    listingInput.value = preFill.listingId;
  }

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
      await call("enquiry.set", {
        conversation: conv.id,
        description: {
          lang: reqLang.value,
          text: desc,
        },
        categories,
        listing_id: listingInput.value.trim() || undefined,
        data_use_notice: dataUseNotice,
      });
      descInput.value = "";
      catsPicker.clear();
      listingInput.value = "";
      reqForm.style.display = "none";
      await redrawNow();
    } catch (err) {
      reqErr.textContent = t("request-send-failed", { detail: errText(err) });
    } finally {
      submitReqBtn.disabled = false;
    }
  };

  reqForm.append(descInput, field(t("request-text-language"), reqLang), catsPicker.root, listingInput, noticeP, submitReqBtn, reqErr);
  requestSection.appendChild(reqForm);
  pane.appendChild(requestSection);
}

export { computeNewestQuoteMap, computeNewestRequestMap, isQuoteSuperseded, newestVersionMap };

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
  newestQuoteMap: Map<string, string>,
  convId: string,
  myDid: string,
  refresh: () => Promise<void>,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = `message message-${m.direction}`;
  wrap.dataset.state = m.state;

  if (m.content_type === "application/vnd.roym.card+json") {
    const cardRow = cardMap.get(m.id);
    if (cardRow?.repeat) {
      wrap.style.display = "none";
      return wrap;
    }
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
      renderCardActions(wrap, cardRow, convId, myDid, refresh, newestQuoteMap);
    } else {
      wrap.appendChild(renderRefusedCard("card", 1, t("messages-unfiled-card")));
    }
  } else if (m.deleted_at_secs != null) {
    wrap.appendChild(text("span", t("messages-message-deleted"), "message-body deleted"));
  } else if (m.body_encoding === "utf8") {
    const nodes = messageText(m.body ?? t("messages-no-body"), m.languages ?? [], "span", {
      translatable: m.direction === "incoming",
    });
    wrap.append(...nodes);
  } else {
    wrap.appendChild(text("span", m.body ?? t("messages-no-body"), "message-body"));
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

/// What the person has on screen for a quote card: the language of the
/// version, and whether it is a machine translation. The card's text
/// element says so (`recordText`); a card with none drawn is read as its
/// original.
function shownVersion(wrap: HTMLElement, card: CardRow): { lang: string; viaMt: boolean } {
  const shown = wrap.querySelector<HTMLElement>("[data-shown-lang]");
  if (shown === null) return { lang: card.data?.terms?.lang ?? "", viaMt: false };
  return { lang: shown.dataset.shownLang ?? "", viaMt: shown.dataset.shownVia === "machine" };
}

/// The warning before an acceptance of terms the person read through
/// machine translation, with a button to go on and one to stop.
function mtWarning(card: CardRow, accept: () => Promise<void>): HTMLElement {
  const box = document.createElement("div");
  box.className = "accept-mt-box";
  box.appendChild(
    text(
      "p",
      t("messages-accept-mt-warning", { language: languageName(card.data?.terms?.lang ?? "") }),
      "accept-mt-warning",
    ),
  );
  const confirmBtn = text("button", t("messages-accept-mt-confirm"), "button accept-mt-confirm") as HTMLButtonElement;
  confirmBtn.onclick = () => {
    box.remove();
    void accept();
  };
  const cancelBtn = text("button", t("common-cancel"), "button accept-mt-cancel") as HTMLButtonElement;
  cancelBtn.onclick = () => box.remove();
  box.append(confirmBtn, cancelBtn);
  return box;
}

export function renderCardActions(
  wrap: HTMLElement,
  card: CardRow,
  convId: string,
  myDid: string,
  refresh: () => Promise<void>,
  newestQuoteMap?: Map<string, string>,
) {
  // Quote this enquiry: on verified enquiry not issued by me
  if (card.card_type === "enquiry" && card.verified) {
    const notIssuedByMe = card.direction === "incoming" || (myDid && card.issuer !== myDid);
    if (notIssuedByMe) {
      const quoteBtn = text("button", t("quote-request-button"), "button quote-request-button") as HTMLButtonElement;
      quoteBtn.onclick = () => openQuoteForm(wrap, card, convId, refresh);
      wrap.appendChild(quoteBtn);
    }
  }

  // Accept / Decline: on verified quote for me, only on newest version of quote_id,
  // not when deal is agreed, declined, expired, or not-formed
  if (card.card_type === "quote" && card.verified) {
    const forMe = !!myDid && card.data?.counterparty_did === myDid;
    const nowSecs = Math.floor(Date.now() / 1000);
    const expSecs = card.data?.terms?.quote_expires_at_secs ?? card.data?.quote_expires_at_secs;
    const isExpired = card.expired || (expSecs !== undefined && nowSecs > expSecs);
    const isDeclined = card.declined === true;

    const quoteId = (card.data as Record<string, unknown> | undefined)?.quote_id as string | undefined;
    const isNewest = !newestQuoteMap || !quoteId || !card.record_id || newestQuoteMap.get(quoteId) === card.record_id;

    const dState = card.deal_state;
    const isEndedOrAgreed = dState
      ? ["agreed", "declined", "expired", "not-formed", "closed", "cancelled"].includes(dState)
      : isExpired || isDeclined;

    if (forMe && isNewest && !isEndedOrAgreed) {
      if (dState === "accepted") {
        wrap.appendChild(text("p", t("messages-waiting-for-other-side"), "quote-waiting-notice"));
        return;
      }
      const actionsDiv = document.createElement("div");
      actionsDiv.className = "quote-actions";

      const acceptBtn = text("button", t("messages-accept-button"), "button accept-quote-button") as HTMLButtonElement;
      const acceptErr = text("p", "", "accept-quote-error");

      const sendAccept = async () => {
        acceptBtn.disabled = true;
        acceptErr.textContent = "";
        const viewed = shownVersion(wrap, card);
        try {
          await call("agreement.accept", {
            quote_record_id: card.record_id,
            ...(viewed.lang ? { viewed_lang: viewed.lang } : {}),
            viewed_via_mt: viewed.viaMt,
          });
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

      // A machine translation is not what binds: say so, and ask again,
      // before anything is signed.
      acceptBtn.onclick = () => {
        if (!shownVersion(wrap, card).viaMt) {
          void sendAccept();
          return;
        }
        if (actionsDiv.querySelector(".accept-mt-box")) return;
        actionsDiv.appendChild(mtWarning(card, sendAccept));
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

  const declineLang = textLangSelect("decline-text-lang", currentLanguage());

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
      await call("deal.decline", {
        declines: card.record_id,
        note: noteInput.value.trim() || undefined,
        lang: declineLang.value || currentLanguage(),
      });
      await refresh();
    } catch (e) {
      err.textContent = t("messages-decline-failed", { detail: errText(e) });
      confirmBtn.disabled = false;
    }
  };

  dialog.append(field(t("request-text-language"), declineLang), noteInput, confirmBtn, cancelBtn, err);
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

  const quoteLang = textLangSelect("quote-text-lang", currentLanguage());

  const scopeInput = document.createElement("input");
  scopeInput.placeholder = t("quote-scope-placeholder");
  scopeInput.className = "quote-scope-input";

  const giveInput = document.createElement("input");
  giveInput.placeholder = t("quote-give-placeholder");
  giveInput.setAttribute("aria-label", t("quote-give-label"));
  giveInput.className = "quote-give-input";

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
      await call("quote.set", {
        enquiry_record_id: reqCard.record_id,
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
    giveInput,
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
    default:
      return err.key;
  }
}

export function readQuoteForm(form: HTMLElement): QuoteFormResult {
  const scopeInput = form.querySelector<HTMLInputElement>(".quote-scope-input");
  const giveInput = form.querySelector<HTMLInputElement>(".quote-give-input");
  const currencyInput = form.querySelector<HTMLInputElement>(".quote-currency-input");
  const amountInput = form.querySelector<HTMLInputElement>(".quote-amount-input");
  const taxInput = form.querySelector<HTMLInputElement>(".quote-tax-input");
  const feesInput = form.querySelector<HTMLInputElement>(".quote-fees-input");
  const payeeInput = form.querySelector<HTMLInputElement>(".quote-payee-input");
  const timingSelect = form.querySelector<HTMLSelectElement>(".quote-timing-select");
  const whereSelect = form.querySelector<HTMLSelectElement>(".quote-where-select");
  const addressInput = form.querySelector<HTMLInputElement>(".quote-address-input");
  const cancInput = form.querySelector<HTMLInputElement>(".quote-canc-input");
  const refInput = form.querySelector<HTMLInputElement>(".quote-ref-input");
  const dispInput = form.querySelector<HTMLInputElement>(".quote-disp-input");
  const quoteLang = form.querySelector<HTMLSelectElement>(".quote-text-lang");

  const scope = scopeInput?.value.trim() ?? "";
  if (!scope) return { error: { key: messageId("quote-scope-required") } };

  const currency = currencyInput?.value.trim().toUpperCase() ?? "";
  if (!currency) return { error: { key: messageId("quote-currency-required") } };

  const exp = currencyMinorExponent(currency);
  if (exp === undefined) {
    return { error: { key: messageId("quote-currency-unknown"), args: { currency } } };
  }

  let amountMinor: number | undefined;
  try {
    amountMinor = toMinorUnits(amountInput?.value ?? "", exp);
  } catch (e) {
    return { error: { raw: errText(e) } };
  }
  if (amountMinor === undefined) return { error: { key: messageId("quote-amount-required") } };

  let taxMinor = 0;
  if (taxInput?.value.trim()) {
    try {
      taxMinor = toMinorUnits(taxInput.value, exp) ?? 0;
    } catch (e) {
      return { error: { raw: errText(e) } };
    }
  }

  let feesMinor = 0;
  if (feesInput?.value.trim()) {
    try {
      feesMinor = toMinorUnits(feesInput.value, exp) ?? 0;
    } catch (e) {
      return { error: { raw: errText(e) } };
    }
  }

  const payee = payeeInput?.value.trim() ?? "";
  if (!payee) return { error: { key: messageId("quote-payee-required") } };

  const where = (whereSelect?.value ?? "at-receiver") as ServiceLocationKind;
  const address = where === "at-receiver" ? addressInput?.value.trim() : undefined;
  const timing = (timingSelect?.value ?? "after-work") as "before-work" | "after-work";
  const lang = quoteLang?.value ?? "en";

  return {
    terms: buildDealTerms(
      {
        give: giveInput?.value.trim() || undefined,
        scope,
        currency,
        amountMinor,
        taxMinor,
        feesMinor,
        payee,
        timing,
        where,
        address,
        cancellation: cancInput?.value.trim(),
        refund: refInput?.value.trim(),
        dispute: dispInput?.value.trim(),
      },
      lang,
    ),
  };
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
