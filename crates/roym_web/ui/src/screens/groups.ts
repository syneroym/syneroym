import { errText, field, refiller, text } from "../dom";
import { onFeed } from "../events";
import { formatDateTime, formatTime } from "../i18n/format";
import { t } from "../i18n/i18n";
import { renderRefusedCard } from "../cards/refused";
import {
  MEMBERSHIP_EVENT_CONTENT_TYPE,
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
  box.appendChild(text("p", t("group-session-ended-notice"), "session-ended-notice"));
  const btn = text("button", t("group-login-again"), "button tab-button") as HTMLButtonElement;
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
    row.appendChild(text("span", t("group-message-deleted"), "deleted-notice"));
    return row;
  }
  const authorName = nameOf(msg.author);
  row.appendChild(text("div", authorName, "message-author"));
  row.appendChild(text("div", msg.body ?? "", "message-body"));

  const meta = document.createElement("div");
  meta.className = "message-meta";
  const dateStr = formatTime(msg.sender_timestamp_ms);
  meta.appendChild(text("span", dateStr, "message-time"));

  if (msg.direction === "outgoing") {
    meta.appendChild(text("span", deliveryWords(msg.state), `message-status status-${msg.state}`));
    if (msg.state === "failed" && onRetry) {
      const retryBtn = text("button", t("group-retry-button"), "button-small retry-button") as HTMLButtonElement;
      retryBtn.onclick = () => onRetry(msg);
      meta.appendChild(retryBtn);
    }
  }

  if (onDelete) {
    const delBtn = text("button", t("group-delete-button"), "button-small delete-button") as HTMLButtonElement;
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
        threadList.appendChild(
          text("div", t("group-profile-named", { author: authorName, name }), "group-event-line"),
        );
      }
      continue;
    }

    if (m.content_type === "application/vnd.roym.card+json") {
      threadList.appendChild(renderRefusedCard("card", 1, t("group-cards-not-allowed")));
      continue;
    }

    threadList.appendChild(renderMessageRow(m, nameOf, onDelete, onRetry));
  }

  if (hasPendingOrFailedOwn) {
    threadList.appendChild(text("p", t("group-delivery-notice"), "group-delivery-notice"));
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
  nameInput.placeholder = t("group-name-placeholder");
  nameInput.className = "input";
  const createBtn = text("button", t("group-new-button"), "button") as HTMLButtonElement;
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

  newGroupBox.append(field(t("group-name-label"), nameInput), createBtn, createErr);
  leftPane.appendChild(newGroupBox);

  const groupItems = document.createElement("div");
  groupItems.className = "conversation-items";
  leftPane.appendChild(groupItems);

  // Hidden groups section
  const hiddenBox = document.createElement("details");
  hiddenBox.className = "hidden-groups-box";
  const hiddenSummary = document.createElement("summary");
  hiddenSummary.textContent = t("group-hidden-summary", { count: 0 });
  const hiddenItems = document.createElement("div");
  hiddenItems.className = "hidden-items";
  hiddenBox.append(hiddenSummary, hiddenItems);
  leftPane.appendChild(hiddenBox);

  type GroupListLoad = { rows: GroupListItem[]; allRows: GroupListItem[] } | "signed-out" | "failed";

  // Resolves to `undefined` when a newer reload replaced this one.
  const fillGroupList = refiller(
    groupItems,
    async (into: DocumentFragment, selectedId?: string): Promise<GroupListLoad> => {
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
        if (err instanceof RpcError && err.type === "NotSignedIn") return "signed-out";
        into.appendChild(text("p", t("common-load-failed", { detail: errText(err) })));
        return "failed";
      }
      if (rows.length === 0) {
        into.appendChild(text("p", t("group-list-empty"), "conversations-empty"));
      }
      for (const r of rows) {
        const row = document.createElement("div");
        row.className = "conversation-item";
        if (r.id === selectedId) row.classList.add("selected");
        const label = r.group?.name || t("group-unnamed");
        row.appendChild(text("div", label, "peer-label"));
        row.appendChild(
          text("div", t("group-message-count", { count: r.message_count }), "count-label"),
        );
        row.onclick = () => {
          groupItems.querySelectorAll(".conversation-item").forEach((el) => el.classList.remove("selected"));
          row.classList.add("selected");
          selectGroup(r.id);
        };
        into.appendChild(row);
      }
      return { rows, allRows };
    },
  );

  async function reloadGroupList(selectedId?: string) {
    const loaded = await fillGroupList(selectedId);
    if (!loaded) return;
    if (loaded === "signed-out") {
      renderSessionEnded(container);
      return;
    }
    hiddenItems.replaceChildren();
    if (loaded === "failed") return;
    const { rows, allRows } = loaded;

    const hiddenList = allRows.filter(
      (r) => r.group?.admission?.state === "hidden" || r.group?.admission?.state === "refused",
    );
    hiddenSummary.textContent = t("group-hidden-summary", { count: hiddenList.length });
    for (const h of hiddenList) {
      const hRow = document.createElement("div");
      hRow.className = "hidden-item";
      const hLabel = h.group?.name || t("group-unnamed");
      hRow.appendChild(text("span", hLabel, "peer-label"));
      const unhideBtn = text("button", t("group-unhide-button"), "button-small") as HTMLButtonElement;
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
      activeGroupId = null;
      activeGroupReload = null;
      rightPane.replaceChildren();
      return;
    }
    const targetId = selectedId && rows.some((r) => r.id === selectedId) ? selectedId : rows[0]?.id;
    if (targetId) {
      await selectGroup(targetId);
    }
  }

  let activeGroupId: string | null = null;
  let activeGroupReload: (() => Promise<void>) | null = null;
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
      rightPane.appendChild(text("p", t("group-load-failed", { detail: errText(err) })));
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
      composerBox.appendChild(text("p", t("group-restored-notice"), "notice-box"));
    } else if (!info.is_member) {
      composerBox.appendChild(text("p", t("group-removed-notice"), "notice-box"));
    } else {
      const sendInput = document.createElement("input");
      sendInput.type = "text";
      sendInput.placeholder = t("group-compose-placeholder");
      sendInput.className = "input";
      const sendBtn = text("button", t("group-send-button"), "button") as HTMLButtonElement;
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

    const titleText = info.name || t("group-unnamed");
    infoPanel.appendChild(text("h3", titleText, "group-title"));

    if (info.is_owner) {
      const renameBox = document.createElement("div");
      renameBox.className = "rename-box";
      const rInput = document.createElement("input");
      rInput.type = "text";
      rInput.placeholder = t("group-rename-placeholder");
      rInput.value = info.name || "";
      rInput.className = "input";
      const rBtn = text("button", t("group-rename-button"), "button-small") as HTMLButtonElement;
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
      text(
        "div",
        info.is_owner
          ? t("group-owner-line-you", { owner: ownerDisplay })
          : t("group-owner-line", { owner: ownerDisplay }),
        "group-owner",
      ),
    );
    infoPanel.appendChild(text("p", t("group-owner-can-read-notice"), "owner-can-read-notice"));

    // Members list
    const membersBox = document.createElement("div");
    membersBox.className = "members-box";
    membersBox.appendChild(text("h4", t("group-members-heading", { count: info.members.length })));
    const membersList = document.createElement("ul");
    membersList.className = "members-list";

    for (const m of info.members) {
      const li = document.createElement("li");
      const mLabel =
        (m.person_did && contactMap.get(m.person_did)) || m.person_did || shortAddress(m.address);
      li.appendChild(text("span", mLabel, "member-name"));
      if (m.is_owner) {
        const badge = text("span", "", "member-badge");
        // Fluent drops a space at the start of a message, so it is added here.
        badge.append(" ", t("group-owner-badge"));
        li.appendChild(badge);
      }
      if (info.is_owner && !m.is_owner) {
        const remBtn = text("button", t("common-remove"), "button-small") as HTMLButtonElement;
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
      addInput.placeholder = t("group-address-placeholder");
      addInput.className = "input";
      const addBtn = text("button", t("group-add-member-button"), "button-small") as HTMLButtonElement;
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
      const dateStr = formatDateTime(info.key_stored_at_ms / 1000);
      infoPanel.appendChild(
        text(
          "div",
          t("group-key-changed", { date: dateStr, epoch: String(info.key_epoch) }),
          "key-changed-date",
        ),
      );
    }
    infoPanel.appendChild(text("p", t("group-key-trust-notice"), "group-key-trust-notice"));
    infoPanel.appendChild(text("p", t("group-join-boundary-notice"), "group-join-boundary-notice"));

    // Transcript check
    const shortDigest = digestStr.length >= 12 ? digestStr.slice(0, 12) : digestStr;
    infoPanel.appendChild(text("div", t("group-transcript-check", { digest: shortDigest }), "transcript-check-code"));
    infoPanel.appendChild(text("p", t("group-transcript-check-notice"), "transcript-check-notice"));

    // Actions
    const actionBox = document.createElement("div");
    actionBox.className = "group-actions";
    const syncBtn = text("button", t("group-sync-button"), "button-small") as HTMLButtonElement;
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

    const hideBtn = text("button", t("group-hide-button"), "button-small") as HTMLButtonElement;
    hideBtn.onclick = async () => {
      if (window.confirm(t("group-hidden-notice"))) {
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

    activeGroupId = gid;
    activeGroupReload = reloadThread;
    await reloadThread();
  }

  const removeFeed = onFeed(async (event) => {
    if (!screen.isConnected) {
      removeFeed();
      return;
    }
    if ("reload" in event && event.reload) {
      if (activeGroupReload) {
        await activeGroupReload();
      }
      await reloadGroupList(activeGroupId ?? undefined);
      return;
    }
    if ("kind" in event) {
      if (activeGroupId && event.subject === activeGroupId && activeGroupReload) {
        await activeGroupReload();
      }
      const hasConv =
        event.kind.startsWith("conversation.") ||
        ("kinds" in event && Boolean(event.kinds?.some((k) => k.startsWith("conversation."))));
      if (hasConv) {
        await reloadGroupList(activeGroupId ?? undefined);
      }
    }
  });

  await reloadGroupList();
}
