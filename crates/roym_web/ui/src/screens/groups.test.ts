import { afterEach, describe, expect, it, vi } from "vitest";
import { MEMBERSHIP_EVENT_CONTENT_TYPE } from "../groups/words";
import { t } from "../i18n/i18n";
import { RpcError } from "../rpc";
import {
  renderGroups,
  renderMessageRow,
  renderThreadMessages,
  type GroupMessageRow,
} from "./groups";
import { stubRpc } from "../testing/stub-rpc";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Groups screen rendering", () => {
  const nameOf = (addr: string) => {
    if (addr === "did:key:zAlice") return "Alice";
    if (addr === "did:key:zBob") return "Bob";
    return addr;
  };

  it("renders a message with markup as a literal text node without executing or creating elements", () => {
    const markupMsg: GroupMessageRow = {
      id: "msg-1",
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      content_type: "text/plain",
      body_encoding: "utf8",
      body: "<img src=x onerror=alert(1)>",
      state: "delivered",
    };

    const row = renderMessageRow(markupMsg, nameOf);
    const bodyEl = row.querySelector(".message-body");
    expect(bodyEl).not.toBeNull();
    expect(bodyEl?.textContent).toBe("<img src=x onerror=alert(1)>");
    expect(bodyEl?.querySelector("img")).toBeNull();
  });

  it("draws every message when one has a timestamp that is not a date", () => {
    const threadList = document.createElement("div");
    const msg = (id: string, sender_timestamp_ms: number): GroupMessageRow => ({
      id,
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms,
      content_type: "text/plain",
      body_encoding: "utf8",
      body: id,
      state: "delivered",
    });
    renderThreadMessages(
      threadList,
      [msg("a", 1000), msg("b", -9223372036854775808), msg("c", 3000)],
      nameOf,
      () => {},
      () => {},
    );
    expect(threadList.querySelectorAll(".message-row")).toHaveLength(3);
    expect(threadList.querySelectorAll(".message-time")[1].textContent).toBe("Invalid Date");
  });

  it("renders membership rows as event lines", () => {
    const threadList = document.createElement("div");
    const msgs: GroupMessageRow[] = [
      {
        id: "ev-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 1000,
        content_type: MEMBERSHIP_EVENT_CONTENT_TYPE,
        body_encoding: "utf8",
        body: JSON.stringify({ action: "add", subject: "did:key:zBob", epoch: 2 }),
        state: "delivered",
      },
    ];

    renderThreadMessages(
      threadList,
      msgs,
      nameOf,
      () => {},
      () => {},
    );
    const events = threadList.querySelectorAll(".group-event-line");
    expect(events.length).toBe(1);
    expect(events[0].textContent).toBe("Alice added Bob");
  });

  it("renders a profile row only when its name differs from the one before it", () => {
    const threadList = document.createElement("div");
    const msgs: GroupMessageRow[] = [
      {
        id: "p-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 1000,
        content_type: "application/vnd.roym.group-profile+json",
        body_encoding: "utf8",
        body: JSON.stringify({ name: "Alpha Team" }),
        state: "delivered",
      },
      {
        id: "p-2",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 2000,
        content_type: "application/vnd.roym.group-profile+json",
        body_encoding: "utf8",
        body: JSON.stringify({ name: "Alpha Team" }), // same name, should be skipped
        state: "delivered",
      },
      {
        id: "p-3",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 3000,
        content_type: "application/vnd.roym.group-profile+json",
        body_encoding: "utf8",
        body: JSON.stringify({ name: "Beta Team" }), // different name, rendered
        state: "delivered",
      },
    ];

    renderThreadMessages(
      threadList,
      msgs,
      nameOf,
      () => {},
      () => {},
    );
    const events = threadList.querySelectorAll(".group-event-line");
    expect(events.length).toBe(2);
    expect(events[0].textContent).toBe("Alice named the group “Alpha Team”");
    expect(events[1].textContent).toBe("Alice named the group “Beta Team”");
  });

  it("renders card rows as a refused/neutral card block only on exact content-type match", () => {
    const threadList = document.createElement("div");
    const msgs: GroupMessageRow[] = [
      {
        id: "card-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 1000,
        content_type: "application/vnd.roym.card+json",
        body_encoding: "utf8",
        body: JSON.stringify({ card_type: "offer" }),
        state: "delivered",
      },
      {
        id: "vcard-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 2000,
        content_type: "text/vcard",
        body_encoding: "utf8",
        body: "BEGIN:VCARD\nFN:Alice\nEND:VCARD",
        state: "delivered",
      },
    ];

    renderThreadMessages(
      threadList,
      msgs,
      nameOf,
      () => {},
      () => {},
    );
    const cards = threadList.querySelectorAll(".card.card-refused");
    expect(cards.length).toBe(1);
    expect(cards[0].textContent).toContain(t("group-cards-not-allowed"));

    const normalBodies = threadList.querySelectorAll(".message-body");
    expect(normalBodies.length).toBe(1);
    expect(normalBodies[0].textContent).toContain("BEGIN:VCARD");
  });

  it("renders the log-in-again state when receiving an RpcError of type NotSignedIn", async () => {
    stubRpc({
      "conversation.list": () => {
        return new RpcError(-32010, "session expired", "NotSignedIn");
      },
    });

    const host = document.createElement("div");
    await renderGroups(host);

    expect(host.textContent).toContain(t("group-session-ended-notice"));
    const reloadBtn = host.querySelector("button");
    expect(reloadBtn?.textContent).toBe("Log in again");
  });

  it("renders restored group with restored notice and omits key date", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 0,
            group: { name: "Restored Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Restored Group",
        owner_address: "did:key:zAlice",
        is_owner: false,
        is_member: false,
        restored_only: true,
        key_epoch: 1,
        key_stored_at_ms: 0,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({ messages: [] }),
    });

    const host = document.createElement("div");
    await renderGroups(host);

    expect(host.textContent).toContain(t("group-restored-notice"));
    expect(host.textContent).not.toContain(t("group-removed-notice"));
    expect(host.querySelector(".key-changed-date")).toBeNull();
  });

  it("retry handler calls conversation.retry with message id and never calls conversation.send", async () => {
    const called: { method: string; params: Record<string, unknown> }[] = [];
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 1,
            group: { name: "Retry Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Retry Group",
        owner_address: "did:key:zAlice",
        is_owner: true,
        is_member: true,
        restored_only: false,
        key_epoch: 1,
        key_stored_at_ms: 0,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({
        messages: [
          {
            id: "msg-failed",
            conversation: "grp-1",
            author: "did:key:zAlice",
            direction: "outgoing",
            sender_timestamp_ms: 1000,
            content_type: "text/plain",
            body_encoding: "utf8",
            body: "failed send",
            state: "failed",
          },
        ],
      }),
      "conversation.retry": (params) => {
        called.push({ method: "conversation.retry", params });
        return { ok: true };
      },
      "conversation.send": (params) => {
        called.push({ method: "conversation.send", params });
        return { ok: true };
      },
    });

    const host = document.createElement("div");
    await renderGroups(host);

    const retryBtn = host.querySelector<HTMLButtonElement>("button.retry-button");
    expect(retryBtn).not.toBeNull();
    retryBtn?.click();
    await new Promise((r) => setTimeout(r, 10));

    expect(called.some((c) => c.method === "conversation.retry")).toBe(true);
    expect(called.find((c) => c.method === "conversation.retry")?.params).toEqual({
      message_id: "msg-failed",
    });
    expect(called.some((c) => c.method === "conversation.send")).toBe(false);
  });

  it("never duplicates group-title or info panel on rapid repeated selection", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 0,
            group: { name: "Garden Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": async () => {
        await new Promise((r) => setTimeout(r, 10));
        return {
          id: "grp-1",
          name: "Garden Group",
          owner_address: "did:key:zAlice",
          is_owner: true,
          is_member: true,
          restored_only: false,
          key_epoch: 1,
          key_stored_at_ms: 0,
          members: [{ address: "did:key:zAlice", is_owner: true }],
        };
      },
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({ messages: [] }),
    });

    const host = document.createElement("div");
    const renderPromise = renderGroups(host);

    await new Promise((r) => setTimeout(r, 2));
    const item = host.querySelector<HTMLElement>(".conversation-item");
    expect(item).not.toBeNull();

    item?.click();
    item?.click();

    await renderPromise;
    await new Promise((r) => setTimeout(r, 50));

    const titles = host.querySelectorAll(".group-title");
    expect(titles.length).toBe(1);
    expect(titles[0].textContent).toBe("Garden Group");
  });

  it("group chat box language list sends languages, keeps rows after send, and allows move up and remove", async () => {
    type SendParams = { conversation: string; body: string; languages?: string[] };
    let lastSendParams: SendParams | undefined;
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 0,
            group: { name: "Garden Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Garden Group",
        owner_address: "did:key:zAlice",
        is_owner: true,
        is_member: true,
        restored_only: false,
        key_epoch: 1,
        key_stored_at_ms: 0,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({ messages: [] }),
      "conversation.send": (params) => {
        lastSendParams = params as SendParams;
        return { message_id: "m_grp_new" };
      },
    });

    const host = document.createElement("div");
    await renderGroups(host);

    // Initial language list has 1 row with English
    let rows = host.querySelectorAll(".group-language-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector(".group-language-name")?.textContent).toBe("English");

    // Send with default ["en"]
    const composerBox = host.querySelector(".composer-box")!;
    const sendInput = composerBox.querySelector<HTMLInputElement>("input")!;
    const sendBtn = composerBox.querySelector<HTMLButtonElement>("button")!;

    sendInput.value = "Hello group";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual(["en"]);

    // After send, list keeps its rows
    rows = host.querySelectorAll(".group-language-row");
    expect(rows).toHaveLength(1);

    // Add Hindi
    const addSel = host.querySelector<HTMLSelectElement>(".group-languages-add")!;
    addSel.value = "hi";
    addSel.dispatchEvent(new Event("change"));

    rows = host.querySelectorAll(".group-language-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".group-language-name")?.textContent).toBe("English");
    expect(rows[1].querySelector(".group-language-name")?.textContent).toBe("Hindi");

    // Move Hindi up
    const upBtn = rows[1].querySelector<HTMLButtonElement>(".group-language-move-up")!;
    upBtn.click();

    rows = host.querySelectorAll(".group-language-row");
    expect(rows[0].querySelector(".group-language-name")?.textContent).toBe("Hindi");
    expect(rows[1].querySelector(".group-language-name")?.textContent).toBe("English");

    // Send with ["hi", "en"]
    lastSendParams = undefined;
    sendInput.value = "नमस्ते समूह";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual(["hi", "en"]);

    // Remove every row
    rows = host.querySelectorAll(".group-language-row");
    rows[0].querySelector<HTMLButtonElement>(".group-language-remove")!.click();
    rows = host.querySelectorAll(".group-language-row");
    rows[0].querySelector<HTMLButtonElement>(".group-language-remove")!.click();

    expect(host.querySelectorAll(".group-language-row")).toHaveLength(0);

    // Send with empty list
    lastSendParams = undefined;
    sendInput.value = "No tag group message";
    sendBtn.click();

    await vi.waitFor(() => expect(lastSendParams).toBeDefined());
    expect((lastSendParams as SendParams | undefined)?.languages).toEqual([]);
  });

  it("click on group language list label or language name does not remove language", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 0,
            group: { name: "Garden Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Garden Group",
        owner_address: "did:key:zAlice",
        is_owner: true,
        is_member: true,
        restored_only: false,
        key_epoch: 1,
        key_stored_at_ms: 0,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({ messages: [] }),
    });
    const host = document.createElement("div");
    await renderGroups(host);

    await vi.waitFor(() => {
      expect(host.querySelectorAll(".group-language-row")).toHaveLength(1);
    });

    let rows = host.querySelectorAll(".group-language-row");
    expect(rows[0].querySelector(".group-language-name")?.textContent).toBe("English");

    // Click on language name
    const langName = rows[0].querySelector<HTMLElement>(".group-language-name")!;
    langName.click();
    rows = host.querySelectorAll(".group-language-row");
    expect(rows).toHaveLength(1);

    // Click on field label text
    const labelSpan = host.querySelector<HTMLElement>(".composer-box .field > span")!;
    expect(labelSpan.textContent).toBe("Message languages");
    labelSpan.click();
    rows = host.querySelectorAll(".group-language-row");
    expect(rows).toHaveLength(1);
  });

  it("renderMessageRow sets lang attribute and note, and omits note for binary messages", () => {
    // 1. Tagged text message
    const msgTa: GroupMessageRow = {
      id: "m-ta",
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      content_type: "text/plain",
      body_encoding: "utf8",
      body: "வணக்கம்",
      languages: ["ta"],
      state: "delivered",
    };
    const rowTa = renderMessageRow(msgTa, nameOf);
    const bodyTa = rowTa.querySelector(".message-body");
    expect(bodyTa?.getAttribute("lang")).toBe("ta");
    expect(rowTa.querySelector(".message-language-note")).toBeNull();

    // 2. Untagged text message
    const msgNoTag: GroupMessageRow = {
      id: "m-notag",
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 2000,
      content_type: "text/plain",
      body_encoding: "utf8",
      body: "plain text",
      state: "delivered",
    };
    const rowNoTag = renderMessageRow(msgNoTag, nameOf);
    const bodyNoTag = rowNoTag.querySelector(".message-body");
    expect(bodyNoTag?.getAttribute("lang")).toBe("");
    const note = rowNoTag.querySelector(".message-language-note");
    expect(note).not.toBeNull();
    expect(note?.textContent).toBe("Language not stated");

    // 3. Binary message
    const msgBin: GroupMessageRow = {
      id: "m-bin",
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 3000,
      content_type: "application/octet-stream",
      body_encoding: "base64",
      body: "AAAA",
      state: "delivered",
    };
    const rowBin = renderMessageRow(msgBin, nameOf);
    const bodyBin = rowBin.querySelector(".message-body");
    expect(bodyBin?.hasAttribute("lang")).toBe(false);
    expect(rowBin.querySelector(".message-language-note")).toBeNull();
  });

  it("membership and group profile name rows show no note", () => {
    const threadList = document.createElement("div");
    const msgs: GroupMessageRow[] = [
      {
        id: "ev-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 1000,
        content_type: MEMBERSHIP_EVENT_CONTENT_TYPE,
        body_encoding: "utf8",
        body: JSON.stringify({ action: "add", subject: "did:key:zBob", epoch: 2 }),
        state: "delivered",
      },
      {
        id: "p-1",
        conversation: "grp-1",
        author: "did:key:zAlice",
        direction: "incoming",
        sender_timestamp_ms: 2000,
        content_type: "application/vnd.roym.group-profile+json",
        body_encoding: "utf8",
        body: JSON.stringify({ name: "New Name" }),
        state: "delivered",
      },
    ];

    renderThreadMessages(threadList, msgs, nameOf, () => {}, () => {});
    expect(threadList.querySelector(".message-language-note")).toBeNull();
  });

  it("draws group members in sortByName order", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-sort",
            kind: "group",
            message_count: 0,
            group: { name: "Sort Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-sort",
        name: "Sort Group",
        owner_address: "did:key:zOwner",
        is_owner: true,
        is_member: true,
        key_epoch: 1,
        key_stored_at_ms: 0,
        members: [
          { address: "addr-deva", person_did: "did:key:zDeva", is_owner: false },
          { address: "addr-latn", person_did: "did:key:zLatn", is_owner: true },
        ],
      }),
      "contacts.list": () => [
        { person_did: "did:key:zDeva", display_name: "राम" },
        { person_did: "did:key:zLatn", display_name: "Alice" },
      ],
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "conversation.history": () => ({ messages: [] }),
    });

    const host = document.createElement("div");
    await renderGroups(host);

    await vi.waitFor(() => {
      const names = Array.from(host.querySelectorAll(".member-name")).map((el) => el.textContent);
      expect(names).toEqual(["Alice", "राम"]);
    });
  });
});
