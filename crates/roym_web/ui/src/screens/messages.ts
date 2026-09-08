import { call, RpcError } from "../rpc";
import { whoami } from "../session/login";
import { renderCard } from "../cards/render";
import { renderRefusedCard } from "../cards/refused";
import { toMinorUnits, currencyMinorExponent } from "../money";

// These two strings are shown to the person *before* a delete runs, and are
// pinned character-for-character against the `note` the conversation
// service returns (`conversation.delete-message`). The product must not
// claim the other side's copy is gone.
const DELETE_NOTE_SENT =
  "The local copy is removed and a deletion record kept. A request to " +
  "delete it was sent to the other side; whether their client honours it " +
  "is theirs to decide, and this cannot check. This installation's own " +
  "message store still holds what it received.";
const DELETE_NOTE_RECEIVED =
  "The local copy is removed and a deletion record kept. This is a " +
  "message you received; the other side's copy is theirs.";

export const DEFAULT_DATA_USE_NOTICE =
  "This request is signed by you and sent to the provider you chose. They " +
  "keep a copy. It carries the area you gave, not your exact address; an " +
  "address is disclosed only inside a quote you accept.";

export const ADDRESS_DISCLOSURE_NOTICE =
  "This address becomes part of a signed record that both parties keep " +
  "and can export. It cannot be removed from a record already signed.";

export const DECLINE_NOTE =
  "This only changes what you see. The other side is not told, and no " +
  "record is signed. Send them a message if you want them to know.";

interface ConversationRow {
  id: string;
  peer_address: string;
  peer_person_did?: string;
  last_activity_ms: number;
  message_count: number;
}

interface MessageRow {
  id: string;
  conversation: string;
  author: string;
  direction: "incoming" | "outgoing";
  sender_timestamp_ms: number;
  content_type: string;
  body_encoding: "utf8" | "base64";
  body?: string;
  state: "pending" | "delivered" | "failed";
  last_error?: string;
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
    consumer_did?: string;
    terms?: {
      quote_expires_at_secs?: number;
    };
    quote_expires_at_secs?: number;
    [key: string]: unknown;
  };
  stored_at_secs: number;
  declined?: boolean;
}

function errText(err: unknown): string {
  if (err instanceof RpcError) return err.message;
  return err instanceof Error ? err.message : String(err);
}

/// Every text node here may carry a stranger's bytes -- an address, a
/// display name, a message body. `textContent` only, never markup.
function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

export async function renderMessages(container: HTMLElement, currentDid?: string) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "messages-screen";

  box.appendChild(text("h2", "Messages"));

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
  openInput.placeholder = "A conversation address (from a listing or a contact)";
  openInput.className = "open-conversation-input";
  const openBtn = text("button", "Open conversation", "button") as HTMLButtonElement;
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
      openErr.textContent = `Could not open: ${errText(err)}`;
    }
    openBtn.disabled = false;
  };
  openRow.append(openInput, openBtn, openErr);
  listPane.appendChild(openRow);

  // --- Search -----------------------------------------------------------
  const searchRow = document.createElement("div");
  searchRow.className = "message-search";
  const searchInput = document.createElement("input");
  searchInput.placeholder = "Search your messages";
  searchInput.className = "message-search-input";
  const searchBtn = text("button", "Search", "button") as HTMLButtonElement;
  const searchResults = document.createElement("div");
  searchResults.className = "search-results";
  searchBtn.onclick = async () => {
    const q = searchInput.value.trim();
    searchResults.replaceChildren();
    if (!q) return;
    try {
      const res = await call<{
        hits: Array<{
          conversation: string;
          message_id: string;
          snippet: string;
        }>;
      }>("conversation.search", { query: q });
      if (res.hits.length === 0) {
        searchResults.appendChild(text("p", "No messages found.", "search-empty"));
        return;
      }
      for (const h of res.hits) {
        const item = document.createElement("div");
        item.className = "search-hit";
        item.appendChild(text("div", h.snippet, "snippet"));
        item.onclick = () => reloadList(h.conversation);
        searchResults.appendChild(item);
      }
    } catch (err) {
      searchResults.appendChild(text("p", `Search failed: ${errText(err)}`));
    }
  };
  searchRow.append(searchInput, searchBtn);
  listPane.append(searchRow, searchResults);

  // --- The list ---------------------------------------------------------
  const listItems = document.createElement("div");
  listItems.className = "conversation-items";
  listPane.appendChild(listItems);

  async function reloadList(select?: string) {
    listItems.replaceChildren();
    let rows: ConversationRow[] = [];
    try {
      const res = await call<{ conversations: ConversationRow[] }>("conversation.list");
      rows = res.conversations;
    } catch (err) {
      listItems.appendChild(text("p", `Could not load: ${errText(err)}`));
      return;
    }
    if (rows.length === 0) {
      listItems.appendChild(text("p", "No conversations yet.", "conversations-empty"));
      threadPane.replaceChildren();
      return;
    }
    for (const r of rows) {
      const row = document.createElement("div");
      row.className = "conversation-item";
      if (r.id === select) row.classList.add("selected");
      row.appendChild(text("div", r.peer_person_did || r.peer_address, "peer-label"));
      row.appendChild(text("div", `${r.message_count} messages`, "count-label"));
      row.onclick = () => {
        listItems.querySelectorAll(".conversation-item").forEach((el) => el.classList.remove("selected"));
        row.classList.add("selected");
        renderThread(threadPane, r, myDid);
      };
      listItems.appendChild(row);
    }
    const target = select ? rows.find((r) => r.id === select) : rows[0];
    if (target) await renderThread(threadPane, target, myDid);
  }

  await reloadList();
  container.appendChild(box);
}

async function renderThread(pane: HTMLElement, conv: ConversationRow, myDid: string) {
  pane.replaceChildren();
  pane.appendChild(text("h3", conv.peer_person_did || conv.peer_address));

  const messagesHost = document.createElement("div");
  messagesHost.className = "thread-messages";
  pane.appendChild(messagesHost);

  // --- Compose message row --------------------------------------------------
  const composeRow = document.createElement("div");
  composeRow.className = "compose-row";
  const composeInput = document.createElement("input");
  composeInput.placeholder = "Write a message";
  composeInput.className = "compose-input";
  const sendBtn = text("button", "Send", "button") as HTMLButtonElement;
  const composeErr = text("p", "", "compose-error");
  sendBtn.onclick = async () => {
    composeErr.textContent = "";
    const body = composeInput.value;
    if (!body.trim()) return;
    sendBtn.disabled = true;
    try {
      await call("conversation.send", { conversation: conv.id, body });
      composeInput.value = "";
      await loadHistory();
    } catch (err) {
      composeErr.textContent = `Send failed: ${errText(err)}`;
    }
    sendBtn.disabled = false;
  };
  composeRow.append(composeInput, sendBtn, composeErr);
  pane.appendChild(composeRow);

  // --- Send a request section -----------------------------------------------
  const requestSection = document.createElement("div");
  requestSection.className = "send-request-section";

  const toggleReqBtn = text("button", "Send a request", "button toggle-request-form") as HTMLButtonElement;
  requestSection.appendChild(toggleReqBtn);

  const reqForm = document.createElement("div");
  reqForm.className = "request-form";
  reqForm.style.display = "none";

  const descInput = document.createElement("textarea");
  descInput.placeholder = "Describe what you need";
  descInput.className = "request-desc-input";

  const catsInput = document.createElement("input");
  catsInput.placeholder = "Categories (comma-separated, e.g. cycling, repair)";
  catsInput.className = "request-cats-input";

  const listingInput = document.createElement("input");
  listingInput.placeholder = "Listing ID (optional)";
  listingInput.className = "request-listing-input";

  const noticeP = text("p", DEFAULT_DATA_USE_NOTICE, "data-use-notice");

  const submitReqBtn = text("button", "Submit request", "button send-request-button") as HTMLButtonElement;
  const reqErr = text("p", "", "request-form-error");

  toggleReqBtn.onclick = () => {
    reqForm.style.display = reqForm.style.display === "none" ? "block" : "none";
  };

  submitReqBtn.onclick = async () => {
    const desc = descInput.value.trim();
    if (!desc) {
      reqErr.textContent = "Description is required";
      return;
    }
    submitReqBtn.disabled = true;
    reqErr.textContent = "";
    try {
      const categories = catsInput.value
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      await call("request.set", {
        conversation: conv.id,
        description: desc,
        categories,
        listing_id: listingInput.value.trim() || undefined,
        data_use_notice: DEFAULT_DATA_USE_NOTICE,
      });
      descInput.value = "";
      catsInput.value = "";
      listingInput.value = "";
      reqForm.style.display = "none";
      await loadHistory();
    } catch (err) {
      reqErr.textContent = `Could not send request: ${errText(err)}`;
    } finally {
      submitReqBtn.disabled = false;
    }
  };

  reqForm.append(descInput, catsInput, listingInput, noticeP, submitReqBtn, reqErr);
  requestSection.appendChild(reqForm);
  pane.appendChild(requestSection);

  async function loadHistory() {
    messagesHost.replaceChildren();

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

    // Map canonical request_id -> newest record_id
    const newestRequestMap = new Map<string, string>();
    for (const c of allCards) {
      if (c.card_type === "request" && c.verified && c.data?.request_id && c.record_id) {
        const reqId = c.data.request_id;
        const currNewestRecord = newestRequestMap.get(reqId);
        if (!currNewestRecord) {
          newestRequestMap.set(reqId, c.record_id);
        } else {
          const existing = allCards.find((x) => x.record_id === currNewestRecord);
          if (existing && (c.data.sequence ?? 0) > (existing.data?.sequence ?? 0)) {
            newestRequestMap.set(reqId, c.record_id);
          }
        }
      }
    }

    let messages: MessageRow[] = [];
    try {
      const res = await call<{ messages: MessageRow[] }>("conversation.history", {
        conversation: conv.id,
      });
      messages = res.messages;
    } catch (err) {
      messagesHost.appendChild(text("p", `Could not load messages: ${errText(err)}`));
      return;
    }

    for (const m of messages) {
      messagesHost.appendChild(
        messageElement(m, cardMap, allCards, newestRequestMap, conv.id, myDid, loadHistory),
      );
    }
  }

  await loadHistory();
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
          }),
        );
      } else if (cardRow.known && !cardRow.verified) {
        wrap.appendChild(renderRefusedCard(cardRow.card_type, cardRow.version, cardRow.reason));
      } else {
        wrap.appendChild(renderCard({ type: cardRow.card_type, version: cardRow.version, data: {} }));
      }

      // Check if quote answers an earlier version of the request
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
            const supP = document.createElement("p");
            supP.className = "quote-superseded-notice";
            supP.textContent = "This quote answers an earlier version of your request.";
            wrap.appendChild(supP);
          }
        }
      }

      // Render contextual card action buttons
      renderCardActions(wrap, cardRow, convId, myDid, refresh);
    } else {
      wrap.appendChild(renderRefusedCard("card", 1, "Unfiled card"));
    }
  } else {
    if (m.deleted_at_secs !== undefined) {
      wrap.appendChild(text("span", "(message deleted)", "message-body deleted"));
    } else {
      wrap.appendChild(text("span", m.body ?? "(no body)", "message-body"));
    }
  }

  // The state word is exactly what the API returned -- never inferred, and
  // "delivered" is never shown until the service says so.
  wrap.appendChild(text("span", m.state, `message-state state-${m.state}`));
  if (m.state === "failed" && m.last_error) {
    wrap.appendChild(text("span", m.last_error, "message-error"));
  }

  const actions = document.createElement("div");
  actions.className = "message-actions";

  if (m.state === "failed") {
    const retryBtn = text("button", "Retry", "button retry-message") as HTMLButtonElement;
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

  if (m.deleted_at_secs === undefined) {
    const deleteBtn = text("button", "Delete", "button delete-message") as HTMLButtonElement;
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
      const quoteBtn = text("button", "Quote this request", "button quote-request-button") as HTMLButtonElement;
      quoteBtn.onclick = () => openQuoteForm(wrap, card, convId, refresh);
      wrap.appendChild(quoteBtn);
    }
  }

  // Accept / Decline: on verified quote for me, not expired, not declined
  if (card.card_type === "quote" && card.verified) {
    const forMe = card.direction === "incoming" || card.data?.consumer_did === myDid;
    const nowSecs = Math.floor(Date.now() / 1000);
    const expSecs = card.data?.terms?.quote_expires_at_secs ?? card.data?.quote_expires_at_secs;
    const isExpired = card.expired || (expSecs !== undefined && nowSecs > expSecs);
    const isDeclined = card.declined === true;

    if (forMe && !isExpired && !isDeclined) {
      const actionsDiv = document.createElement("div");
      actionsDiv.className = "quote-actions";

      const acceptBtn = text("button", "Accept these terms", "button accept-quote-button") as HTMLButtonElement;
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
            acceptErr.textContent = "This quote has expired. Ask for a new one.";
          } else {
            acceptErr.textContent = `Could not accept: ${msg}`;
          }
        }
      };

      const declineBtn = text("button", "Decline", "button decline-quote-button") as HTMLButtonElement;
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
  dialog.appendChild(text("p", DECLINE_NOTE, "decline-note"));

  const noteInput = document.createElement("input");
  noteInput.placeholder = "Optional note (for yourself only)";
  noteInput.className = "decline-note-input";

  const confirmBtn = text("button", "Confirm Decline", "button confirm-decline") as HTMLButtonElement;
  const cancelBtn = text("button", "Cancel", "button cancel-decline") as HTMLButtonElement;
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
      err.textContent = `Decline failed: ${errText(e)}`;
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
  scopeInput.placeholder = "Scope of work";
  scopeInput.className = "quote-scope-input";

  const currencyInput = document.createElement("input");
  currencyInput.placeholder = "Currency (e.g. EUR, USD)";
  currencyInput.defaultValue = "EUR";
  currencyInput.className = "quote-currency-input";

  const amountInput = document.createElement("input");
  amountInput.placeholder = "Total amount (e.g. 45.00)";
  amountInput.className = "quote-amount-input";

  const taxInput = document.createElement("input");
  taxInput.placeholder = "Tax included (optional, e.g. 5.00)";
  taxInput.className = "quote-tax-input";

  const feesInput = document.createElement("input");
  feesInput.placeholder = "Fees included (optional, e.g. 0.00)";
  feesInput.className = "quote-fees-input";

  const payeeInput = document.createElement("input");
  payeeInput.placeholder = "Payee name or company";
  payeeInput.className = "quote-payee-input";

  const timingSelect = document.createElement("select");
  timingSelect.className = "quote-timing-select";
  const optAfter = document.createElement("option");
  optAfter.value = "after-work";
  optAfter.textContent = "Payment after work";
  const optBefore = document.createElement("option");
  optBefore.value = "before-work";
  optBefore.textContent = "Payment before work";
  timingSelect.append(optAfter, optBefore);

  const whereSelect = document.createElement("select");
  whereSelect.className = "quote-where-select";
  const optCust = document.createElement("option");
  optCust.value = "at-customer";
  optCust.textContent = "At customer";
  const optProv = document.createElement("option");
  optProv.value = "at-provider";
  optProv.textContent = "At provider";
  const optRem = document.createElement("option");
  optRem.value = "remote";
  optRem.textContent = "Remote";
  whereSelect.append(optCust, optProv, optRem);

  const addressWrap = document.createElement("div");
  addressWrap.className = "quote-address-wrap";
  const addressNotice = text("p", ADDRESS_DISCLOSURE_NOTICE, "address-disclosure-notice");
  const addressInput = document.createElement("input");
  addressInput.placeholder = "Service address";
  addressInput.className = "quote-address-input";
  addressWrap.append(addressNotice, addressInput);

  whereSelect.onchange = () => {
    addressWrap.style.display = whereSelect.value === "at-customer" ? "block" : "none";
  };

  const cancInput = document.createElement("input");
  cancInput.placeholder = "Cancellation terms";
  cancInput.defaultValue = "24 hours notice required for full refund";
  cancInput.className = "quote-canc-input";

  const refInput = document.createElement("input");
  refInput.placeholder = "Refund terms";
  refInput.defaultValue = "Full refund if work not completed";
  refInput.className = "quote-ref-input";

  const dispInput = document.createElement("input");
  dispInput.placeholder = "Dispute path";
  dispInput.defaultValue = "Small claims court or informal mediation";
  dispInput.className = "quote-disp-input";

  const hoursInput = document.createElement("input");
  hoursInput.placeholder = "Expires in hours";
  hoursInput.defaultValue = "24";
  hoursInput.type = "number";
  hoursInput.className = "quote-hours-input";

  const submitBtn = text("button", "Send quote", "button submit-quote-button") as HTMLButtonElement;
  const cancelBtn = text("button", "Cancel", "button cancel-quote-button") as HTMLButtonElement;
  const errP = text("p", "", "quote-form-error");

  cancelBtn.onclick = () => form.remove();

  submitBtn.onclick = async () => {
    errP.textContent = "";
    const scope = scopeInput.value.trim();
    if (!scope) {
      errP.textContent = "Scope is required";
      return;
    }
    const currency = currencyInput.value.trim().toUpperCase();
    if (!currency) {
      errP.textContent = "Currency is required";
      return;
    }
    let amountMinor: number | undefined;
    try {
      amountMinor = toMinorUnits(amountInput.value, currencyMinorExponent(currency));
    } catch (e) {
      errP.textContent = errText(e);
      return;
    }
    if (amountMinor === undefined) {
      errP.textContent = "Amount is required";
      return;
    }

    let taxMinor = 0;
    if (taxInput.value.trim()) {
      try {
        taxMinor = toMinorUnits(taxInput.value, currencyMinorExponent(currency)) ?? 0;
      } catch (e) {
        errP.textContent = errText(e);
        return;
      }
    }

    let feesMinor = 0;
    if (feesInput.value.trim()) {
      try {
        feesMinor = toMinorUnits(feesInput.value, currencyMinorExponent(currency)) ?? 0;
      } catch (e) {
        errP.textContent = errText(e);
        return;
      }
    }

    const payee = payeeInput.value.trim();
    if (!payee) {
      errP.textContent = "Payee is required";
      return;
    }

    const hours = Number.parseInt(hoursInput.value || "24", 10);
    const expires_in_secs = Math.max(300, hours * 3600);

    const where = whereSelect.value as "at-customer" | "at-provider" | "remote";
    const address = where === "at-customer" ? addressInput.value.trim() : undefined;

    submitBtn.disabled = true;
    try {
      await call("quote.set", {
        request_record_id: reqCard.record_id,
        expires_in_secs,
        terms: {
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
      errP.textContent = `Quote failed: ${errText(err)}`;
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
  dialog.appendChild(text("p", isSent ? DELETE_NOTE_SENT : DELETE_NOTE_RECEIVED, "delete-note"));

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
    label.append(cb, text("span", "Also ask them to delete their copy"));
    dialog.appendChild(label);
  }

  const confirmBtn = text("button", "Delete this copy", "button confirm-delete") as HTMLButtonElement;
  const cancelBtn = text("button", "Cancel", "button cancel-delete") as HTMLButtonElement;
  const err = text("p", "", "delete-error");
  cancelBtn.onclick = () => dialog.remove();
  confirmBtn.onclick = async () => {
    confirmBtn.disabled = true;
    try {
      await call("conversation.delete-message", { message_id: m.id, ask_peer: askPeer });
      await refresh();
    } catch (e) {
      err.textContent = `Delete failed: ${errText(e)}`;
      confirmBtn.disabled = false;
    }
  };
  dialog.append(confirmBtn, cancelBtn, err);
  anchor.appendChild(dialog);
}
