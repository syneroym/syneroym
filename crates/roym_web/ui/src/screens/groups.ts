import { errText, field, text } from "../dom";
import { renderRefusedCard } from "../cards/refused";
import {
  CARDS_NOT_IN_GROUPS_MESSAGE,
  GROUP_DELIVERY_NOTICE,
  GROUP_HIDDEN_NOTICE,
  GROUP_JOIN_BOUNDARY_NOTICE,
  GROUP_KEY_TRUST_NOTICE,
  GROUP_REMOVED_NOTICE,
  GROUP_RESTORED_NOTICE,
  MEMBERSHIP_EVENT_CONTENT_TYPE,
  OWNER_CAN_READ_NOTICE,
  SESSION_ENDED_NOTICE,
  TRANSCRIPT_CHECK_NOTICE,
  deliveryWords,
  membershipEventWords,
  shortAddress,
  type MembershipEventPayload,
} from "../groups/words";
import { call, RpcError } from "../rpc";

export interface GroupListItem {
  id: string;
  kind?: "direct" | "group";
  peer_address: string;
  last_activity_ms: number;
  message_count: number;
  group?: {
    name?: string | null;
    admission?: {
      state: "shown" | "hidden" | "refused";
      reason?: string;
    };
  };
}

export interface GroupMemberInfo {
  address: string;
  person_did?: string | null;
  is_owner: boolean;
}

export interface GroupDetails {
  conversation_id: string;
  name?: string | null;
  owner_address: string;
  owner_person_did?: string | null;
  is_owner: boolean;
  is_member: boolean;
  restored_only?: boolean;
  epoch: number;
  key_epoch: number;
  key_stored_at_ms: number;
  members: GroupMemberInfo[];
  can_read_new_messages: boolean;
  admission?: {
    state: "shown" | "hidden" | "refused";
    reason?: string;
  };
}

export interface GroupMessageRow {
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

export function parseProfileName(body?: string): string | null {
  if (!body) return null;
  try {
    const parsed = JSON.parse(body);
    return typeof parsed.name === "string" ? parsed.name : null;
  } catch {
    return null;
  }
}

export function parseMembershipEvent(body?: string): MembershipEventPayload | null {
  if (!body) return null;
  try {
    return JSON.parse(body);
  } catch {
    return null;
  }
}

export function renderSessionEnded(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "session-ended-box";
  box.appendChild(text("p", SESSION_ENDED_NOTICE, "session-ended-notice"));
  const btn = text("button", "Log in again", "button tab-button") as HTMLButtonElement;
  btn.onclick = () => window.location.reload();
  box.appendChild(btn);
  container.appendChild(box);
}

export function renderMessageRow(
  msg: GroupMessageRow,
  nameOf: (addr: string) => string,
  onDelete?: (id: string) => void,
  onRetry?: (msg: GroupMessageRow) => void,
): HTMLElement {
  const row = document.createElement("div");
  row.className = `message-row message-${msg.direction}`;
  if (msg.deleted_at_secs) {
    row.classList.add("message-deleted");
    row.appendChild(text("span", "This message was deleted.", "deleted-notice"));
    return row;
  }
  const authorName = nameOf(msg.author);
  row.appendChild(text("div", authorName, "message-author"));
  row.appendChild(text("div", msg.body ?? "", "message-body"));

  const meta = document.createElement("div");
  meta.className = "message-meta";
  const dateStr = new Date(msg.sender_timestamp_ms).toLocaleTimeString();
  meta.appendChild(text("span", dateStr, "message-time"));

  if (msg.direction === "outgoing") {
    meta.appendChild(text("span", deliveryWords(msg.state), `message-status status-${msg.state}`));
    if (msg.state === "failed" && onRetry) {
      const retryBtn = text("button", "Retry", "button-small retry-button") as HTMLButtonElement;
      retryBtn.onclick = () => onRetry(msg);
      meta.appendChild(retryBtn);
    }
  }

  if (onDelete) {
    const delBtn = text("button", "Delete", "button-small delete-button") as HTMLButtonElement;
    delBtn.onclick = () => onDelete(msg.id);
    meta.appendChild(delBtn);
  }

  row.appendChild(meta);
  return row;
}

export function renderThreadMessages(
  threadList: HTMLElement,
  messages: GroupMessageRow[],
  nameOf: (addr: string) => string,
  onDelete: (id: string) => void,
  onRetry: (msg: GroupMessageRow) => void,
) {
  threadList.replaceChildren();
  let lastProfileName: string | null = null;
  let hasPendingOrFailedOwn = false;

  for (const m of messages) {
    if (m.direction === "outgoing" && (m.state === "pending" || m.state === "failed")) {
      hasPendingOrFailedOwn = true;
    }

    if (m.content_type === MEMBERSHIP_EVENT_CONTENT_TYPE) {
      const ev = parseMembershipEvent(m.body);
      if (ev) {
        const words = membershipEventWords(ev, nameOf, m.author);
        threadList.appendChild(text("div", words, "group-event-line"));
      }
      continue;
    }

    if (m.content_type === "application/vnd.roym.group-profile+json") {
      const name = parseProfileName(m.body);
      if (name && name !== lastProfileName) {
        lastProfileName = name;
        const authorName = nameOf(m.author);
        threadList.appendChild(text("div", `${authorName} named the group “${name}”`, "group-event-line"));
      }
      continue;
    }

    if (m.content_type === "application/vnd.roym.card+json") {
      threadList.appendChild(renderRefusedCard("card", 1, CARDS_NOT_IN_GROUPS_MESSAGE));
      continue;
    }

    threadList.appendChild(renderMessageRow(m, nameOf, onDelete, onRetry));
  }

  if (hasPendingOrFailedOwn) {
    threadList.appendChild(text("p", GROUP_DELIVERY_NOTICE, "group-delivery-notice"));
  }
}

export async function renderGroups(container: HTMLElement) {
  container.replaceChildren();
  const screen = document.createElement("div");
  screen.className = "groups-screen messages-screen";

  const leftPane = document.createElement("div");
  leftPane.className = "conversations-pane";

  const rightPane = document.createElement("div");
  rightPane.className = "thread-and-info-pane";

  screen.append(leftPane, rightPane);
  container.appendChild(screen);

  // New Group Form
  const newGroupBox = document.createElement("div");
  newGroupBox.className = "new-group-box";
  const nameInput = document.createElement("input");
  nameInput.type = "text";
  nameInput.placeholder = "Group name";
  nameInput.className = "input";
  const createBtn = text("button", "New group", "button") as HTMLButtonElement;
  const createErr = document.createElement("div");
  createErr.className = "error-text";

  createBtn.onclick = async () => {
    createErr.textContent = "";
    const name = nameInput.value.trim();
    try {
      const res = await call<{ conversation_id: string }>("group.create", name ? { name } : {});
      nameInput.value = "";
      await reloadGroupList(res.conversation_id);
    } catch (err) {
      if (err instanceof RpcError && err.type === "NotSignedIn") {
        renderSessionEnded(container);
        return;
      }
      createErr.textContent = errText(err);
    }
  };

  newGroupBox.append(field("Group Name", nameInput), createBtn, createErr);
  leftPane.appendChild(newGroupBox);

  const groupItems = document.createElement("div");
  groupItems.className = "conversation-items";
  leftPane.appendChild(groupItems);

  // Hidden groups section
  const hiddenBox = document.createElement("details");
  hiddenBox.className = "hidden-groups-box";
  const hiddenSummary = document.createElement("summary");
  hiddenSummary.textContent = "Hidden groups (0)";
  const hiddenItems = document.createElement("div");
  hiddenItems.className = "hidden-items";
  hiddenBox.append(hiddenSummary, hiddenItems);
  leftPane.appendChild(hiddenBox);

  async function reloadGroupList(selectedId?: string) {
    groupItems.replaceChildren();
    hiddenItems.replaceChildren();
    let rows: GroupListItem[] = [];
    let allRows: GroupListItem[] = [];
    try {
      const res = await call<{ conversations: GroupListItem[] }>("conversation.list", { kind: "group" });
      rows = res.conversations || [];
      const hiddenRes = await call<{ conversations: GroupListItem[] }>("conversation.list", {
        kind: "group",
        include_hidden: true,
      });
      allRows = hiddenRes.conversations || [];
    } catch (err) {
      if (err instanceof RpcError && err.type === "NotSignedIn") {
        renderSessionEnded(container);
        return;
      }
      groupItems.appendChild(text("p", `Could not load: ${errText(err)}`));
      return;
    }

    const hiddenList = allRows.filter(
      (r) => r.group?.admission?.state === "hidden" || r.group?.admission?.state === "refused",
    );
    hiddenSummary.textContent = `Hidden groups (${hiddenList.length})`;
    for (const h of hiddenList) {
      const hRow = document.createElement("div");
      hRow.className = "hidden-item";
      const hLabel = h.group?.name || "Unnamed group";
      hRow.appendChild(text("span", hLabel, "peer-label"));
      const unhideBtn = text("button", "Unhide", "button-small") as HTMLButtonElement;
      unhideBtn.onclick = async () => {
        try {
          await call("group.unhide", { conversation: h.id });
          await reloadGroupList(h.id);
        } catch (err) {
          if (err instanceof RpcError && err.type === "NotSignedIn") {
            renderSessionEnded(container);
            return;
          }
          alert(errText(err));
        }
      };
      hRow.appendChild(unhideBtn);
      hiddenItems.appendChild(hRow);
    }

    if (rows.length === 0) {
      groupItems.appendChild(text("p", "No groups yet.", "conversations-empty"));
      rightPane.replaceChildren();
      return;
    }

    for (const r of rows) {
      const row = document.createElement("div");
      row.className = "conversation-item";
      if (r.id === selectedId) row.classList.add("selected");
      const label = r.group?.name || "Unnamed group";
      row.appendChild(text("div", label, "peer-label"));
      row.appendChild(text("div", `${r.message_count} messages`, "count-label"));
      row.onclick = () => {
        groupItems.querySelectorAll(".conversation-item").forEach((el) => el.classList.remove("selected"));
        row.classList.add("selected");
        selectGroup(r.id);
      };
      groupItems.appendChild(row);
    }

    const targetId = selectedId && rows.some((r) => r.id === selectedId) ? selectedId : rows[0]?.id;
    if (targetId) {
      await selectGroup(targetId);
    }
  }

  let currentSelectionSeq = 0;

  async function selectGroup(gid: string) {
    const seq = ++currentSelectionSeq;
    rightPane.replaceChildren();
    let info: GroupDetails;
    try {
      info = await call<GroupDetails>("group.info", { conversation: gid });
    } catch (err) {
      if (seq !== currentSelectionSeq) return;
      if (err instanceof RpcError && err.type === "NotSignedIn") {
        renderSessionEnded(container);
        return;
      }
      rightPane.appendChild(text("p", `Could not load group: ${errText(err)}`));
      return;
    }

    let digestStr = "";
    try {
      const digRes = await call<{ digest: string }>("conversation.transcript-digest", { conversation: gid });
      digestStr = digRes.digest || "";
    } catch {
      /* non-fatal */
    }

    const contactMap = new Map<string, string>();
    try {
      const contacts =
        await call<Array<{ person_did: string; display_name?: string }>>("contacts.list");
      for (const c of contacts) {
        if (c.person_did) {
          contactMap.set(c.person_did, c.display_name || "");
        }
      }
    } catch {
      /* non-fatal */
    }

    const memberMap = new Map<string, string>();
    for (const m of info.members) {
      if (m.person_did) {
        memberMap.set(m.address, m.person_did);
      }
    }
    const nameOf = (addr: string) => {
      const did = memberMap.get(addr);
      if (did) {
        return contactMap.get(did) || did;
      }
      return shortAddress(addr);
    };

    // Thread Pane
    const threadCol = document.createElement("div");
    threadCol.className = "thread-pane";

    const threadList = document.createElement("div");
    threadList.className = "message-list";
    threadCol.appendChild(threadList);

    // Composer
    const composerBox = document.createElement("div");
    composerBox.className = "composer-box";

    if (info.restored_only) {
      composerBox.appendChild(text("p", GROUP_RESTORED_NOTICE, "notice-box"));
    } else if (!info.is_member) {
      composerBox.appendChild(text("p", GROUP_REMOVED_NOTICE, "notice-box"));
    } else {
      const sendInput = document.createElement("input");
      sendInput.type = "text";
      sendInput.placeholder = "Write a message...";
      sendInput.className = "input";
      const sendBtn = text("button", "Send", "button") as HTMLButtonElement;
      const sendErr = document.createElement("div");
      sendErr.className = "error-text composer-error";

      const doSend = async () => {
        const body = sendInput.value.trim();
        if (!body) return;
        sendErr.textContent = "";
        try {
          await call("conversation.send", { conversation: gid, body });
          sendInput.value = "";
          await reloadThread();
        } catch (err) {
          if (err instanceof RpcError && err.type === "NotSignedIn") {
            renderSessionEnded(container);
            return;
          }
          sendErr.textContent = errText(err);
        }
      };

      sendBtn.onclick = doSend;
      sendInput.onkeydown = (e) => {
        if (e.key === "Enter") doSend();
      };
      composerBox.append(sendInput, sendBtn, sendErr);
    }
    threadCol.appendChild(composerBox);

    // Info Panel
    const infoPanel = document.createElement("div");
    infoPanel.className = "group-info-panel";

    const titleText = info.name || "Unnamed group";
    infoPanel.appendChild(text("h3", titleText, "group-title"));

    if (info.is_owner) {
      const renameBox = document.createElement("div");
      renameBox.className = "rename-box";
      const rInput = document.createElement("input");
      rInput.type = "text";
      rInput.placeholder = "New name";
      rInput.value = info.name || "";
      rInput.className = "input";
      const rBtn = text("button", "Rename", "button-small") as HTMLButtonElement;
      const rErr = document.createElement("div");
      rErr.className = "error-text";
      rBtn.onclick = async () => {
        rErr.textContent = "";
        try {
          await call("group.rename", { conversation: gid, name: rInput.value.trim() });
          await reloadGroupList(gid);
        } catch (err) {
          if (err instanceof RpcError && err.type === "NotSignedIn") {
            renderSessionEnded(container);
            return;
          }
          rErr.textContent = errText(err);
        }
      };
      renameBox.append(rInput, rBtn, rErr);
      infoPanel.appendChild(renameBox);
    }

    const ownerDisplay = nameOf(info.owner_address);
    infoPanel.appendChild(
      text("div", `Owner: ${ownerDisplay}${info.is_owner ? " (you)" : ""}`, "group-owner"),
    );
    infoPanel.appendChild(text("p", OWNER_CAN_READ_NOTICE, "owner-can-read-notice"));

    // Members list
    const membersBox = document.createElement("div");
    membersBox.className = "members-box";
    membersBox.appendChild(text("h4", `Members (${info.members.length})`));
    const membersList = document.createElement("ul");
    membersList.className = "members-list";

    for (const m of info.members) {
      const li = document.createElement("li");
      const mLabel =
        (m.person_did && contactMap.get(m.person_did)) || m.person_did || shortAddress(m.address);
      li.appendChild(text("span", mLabel, "member-name"));
      if (m.is_owner) {
        li.appendChild(text("span", " (owner)", "member-badge"));
      }
      if (info.is_owner && !m.is_owner) {
        const remBtn = text("button", "Remove", "button-small") as HTMLButtonElement;
        remBtn.onclick = async () => {
          try {
            await call("group.remove-member", { conversation: gid, address: m.address });
            await selectGroup(gid);
          } catch (err) {
            if (err instanceof RpcError && err.type === "NotSignedIn") {
              renderSessionEnded(container);
              return;
            }
            alert(errText(err));
          }
        };
        li.appendChild(remBtn);
      }
      membersList.appendChild(li);
    }
    membersBox.appendChild(membersList);

    if (info.is_owner) {
      const addBox = document.createElement("div");
      addBox.className = "add-member-box";
      const addInput = document.createElement("input");
      addInput.type = "text";
      addInput.placeholder = "Address or Person DID";
      addInput.className = "input";
      const addBtn = text("button", "Add member", "button-small") as HTMLButtonElement;
      const addErr = document.createElement("div");
      addErr.className = "error-text";
      addBtn.onclick = async () => {
        addErr.textContent = "";
        const target = addInput.value.trim();
        if (!target) return;
        try {
          if (target.startsWith("did:") && contactMap.has(target)) {
            await call("group.add-member", { conversation: gid, person_did: target });
          } else {
            await call("group.add-member", { conversation: gid, address: target });
          }
          addInput.value = "";
          await selectGroup(gid);
        } catch (err) {
          if (err instanceof RpcError && err.type === "NotSignedIn") {
            renderSessionEnded(container);
            return;
          }
          addErr.textContent = errText(err);
        }
      };
      addBox.append(addInput, addBtn, addErr);
      membersBox.appendChild(addBox);
    }
    infoPanel.appendChild(membersBox);

    // Key trust & boundaries
    if (!info.restored_only && info.key_stored_at_ms > 0) {
      const dateStr = new Date(info.key_stored_at_ms).toLocaleString();
      infoPanel.appendChild(
        text("div", `Group key changed here: ${dateStr} (epoch ${info.key_epoch})`, "key-changed-date"),
      );
    }
    infoPanel.appendChild(text("p", GROUP_KEY_TRUST_NOTICE, "group-key-trust-notice"));
    infoPanel.appendChild(text("p", GROUP_JOIN_BOUNDARY_NOTICE, "group-join-boundary-notice"));

    // Transcript check
    const shortDigest = digestStr.length >= 12 ? digestStr.slice(0, 12) : digestStr;
    infoPanel.appendChild(text("div", `Transcript check: ${shortDigest}`, "transcript-check-code"));
    infoPanel.appendChild(text("p", TRANSCRIPT_CHECK_NOTICE, "transcript-check-notice"));

    // Actions
    const actionBox = document.createElement("div");
    actionBox.className = "group-actions";
    const syncBtn = text("button", "Sync now", "button-small") as HTMLButtonElement;
    syncBtn.onclick = async () => {
      try {
        await call("group.sync", { conversation: gid });
        await reloadThread();
      } catch (err) {
        if (err instanceof RpcError && err.type === "NotSignedIn") {
          renderSessionEnded(container);
          return;
        }
        alert(errText(err));
      }
    };

    const hideBtn = text("button", "Hide this group", "button-small") as HTMLButtonElement;
    hideBtn.onclick = async () => {
      if (window.confirm(GROUP_HIDDEN_NOTICE)) {
        try {
          await call("group.hide", { conversation: gid });
          await reloadGroupList();
        } catch (err) {
          if (err instanceof RpcError && err.type === "NotSignedIn") {
            renderSessionEnded(container);
            return;
          }
          alert(errText(err));
        }
      }
    };
    actionBox.append(syncBtn, hideBtn);
    infoPanel.appendChild(actionBox);

    if (seq !== currentSelectionSeq) return;
    rightPane.replaceChildren(threadCol, infoPanel);

    async function reloadThread() {
      if (seq !== currentSelectionSeq) return;
      try {
        const hist = await call<{ messages: GroupMessageRow[] }>("conversation.history", {
          conversation: gid,
        });
        if (seq !== currentSelectionSeq) return;
        const msgs = hist.messages || [];
        renderThreadMessages(
          threadList,
          msgs,
          nameOf,
          async (msgId) => {
            try {
              await call("conversation.delete-message", { message_id: msgId, ask_peer: true });
              await reloadThread();
            } catch (err) {
              if (err instanceof RpcError && err.type === "NotSignedIn") {
                renderSessionEnded(container);
              }
            }
          },
          async (m) => {
            try {
              await call("conversation.retry", { message_id: m.id });
              await reloadThread();
            } catch (err) {
              if (err instanceof RpcError && err.type === "NotSignedIn") {
                renderSessionEnded(container);
              }
            }
          },
        );
      } catch (err) {
        if (err instanceof RpcError && err.type === "NotSignedIn") {
          renderSessionEnded(container);
        }
      }
    }

    await reloadThread();
  }

  await reloadGroupList();
}
