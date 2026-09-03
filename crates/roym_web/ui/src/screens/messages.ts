import { call, RpcError } from "../rpc";

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

export async function renderMessages(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "messages-screen";

  box.appendChild(text("h2", "Messages"));

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
  openInput.placeholder = "Person DID or conversation address";
  const openBtn = text("button", "Open conversation", "button") as HTMLButtonElement;
  const openErr = text("p", "", "open-error");
  openBtn.onclick = async () => {
    openErr.textContent = "";
    const v = openInput.value.trim();
    if (!v) return;
    try {
      const params = v.startsWith("did:key:") ? { person_did: v } : { address: v };
      const res = await call<{ conversation_id: string }>("conversation.open", params);
      openInput.value = "";
      await reloadList(res.conversation_id);
    } catch (err) {
      openErr.textContent = `Could not open: ${errText(err)}`;
    }
  };
  openRow.append(openInput, openBtn, openErr);
  listPane.appendChild(openRow);

  // --- Search -----------------------------------------------------------
  const searchRow = document.createElement("div");
  searchRow.className = "message-search";
  const searchInput = document.createElement("input");
  searchInput.placeholder = "Search your messages";
  const searchBtn = text("button", "Search", "button") as HTMLButtonElement;
  const searchResults = document.createElement("div");
  searchResults.className = "search-results";
  searchBtn.onclick = async () => {
    searchResults.replaceChildren();
    const q = searchInput.value.trim();
    if (!q) return;
    try {
      const res = await call<{ matches: MessageRow[] }>("conversation.search", { query: q });
      if (res.matches.length === 0) {
        searchResults.appendChild(text("p", "No messages matched."));
        return;
      }
      for (const m of res.matches) {
        searchResults.appendChild(text("div", m.body ?? "(no body)", "search-hit"));
      }
    } catch (err) {
      searchResults.appendChild(text("p", `Search failed: ${errText(err)}`));
    }
  };
  searchRow.append(searchInput, searchBtn, searchResults);
  listPane.appendChild(searchRow);

  const rowsHost = document.createElement("div");
  rowsHost.className = "conversation-rows";
  listPane.appendChild(rowsHost);

  async function reloadList(select?: string) {
    rowsHost.replaceChildren();
    let rows: ConversationRow[] = [];
    try {
      const res = await call<{ conversations: ConversationRow[] }>("conversation.list");
      rows = res.conversations;
    } catch (err) {
      rowsHost.appendChild(text("p", `Could not load conversations: ${errText(err)}`));
      return;
    }
    if (rows.length === 0) {
      rowsHost.appendChild(text("p", "No conversations yet."));
    }
    for (const row of rows) {
      const btn = text(
        "button",
        `${row.peer_person_did || row.peer_address} (${row.message_count})`,
        "button conversation-row",
      ) as HTMLButtonElement;
      btn.dataset.conversation = row.id;
      btn.onclick = () => renderThread(threadPane, row);
      rowsHost.appendChild(btn);
    }
    const target = select ? rows.find((r) => r.id === select) : rows[0];
    if (target) await renderThread(threadPane, target);
  }

  await reloadList();
  container.appendChild(box);
}

async function renderThread(pane: HTMLElement, conv: ConversationRow) {
  pane.replaceChildren();
  pane.appendChild(text("h3", conv.peer_person_did || conv.peer_address));

  const messagesHost = document.createElement("div");
  messagesHost.className = "thread-messages";
  pane.appendChild(messagesHost);

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

  async function loadHistory() {
    messagesHost.replaceChildren();
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
      messagesHost.appendChild(messageElement(m, loadHistory));
    }
  }

  await loadHistory();
}

function messageElement(m: MessageRow, refresh: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = `message message-${m.direction}`;
  wrap.dataset.state = m.state;

  if (m.deleted_at_secs !== undefined) {
    wrap.appendChild(text("span", "(message deleted)", "message-body deleted"));
  } else {
    wrap.appendChild(text("span", m.body ?? "(no body)", "message-body"));
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
