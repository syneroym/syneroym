import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MEMBERSHIP_EVENT_CONTENT_TYPE } from "../groups/words";
import { _resetForTest, startFeed } from "../events";
import { FakeFeedSource } from "../testing/fake-event-source";
import { t } from "../i18n/i18n";
import { RpcError } from "../rpc";
import {
  renderGroups,
  renderMessageRow,
  renderThreadMessages,
  type GroupMessageRow,
} from "./groups";
import { pollDefinition, type WidgetView } from "../widgets/view";
import { stubRpc } from "../testing/stub-rpc";
import { readerReads } from "../testing/translation";
import { setTargetsForTest } from "../translation/translation";
import { resetForwardSettingsCache } from "../chat/forward_mode";

afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  resetForwardSettingsCache();
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
    type SendParams = {
      conversation: string;
      body: string;
      languages?: string[];
      forward_mode?: string;
    };
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
      "conversation.settings": () => ({ default_mode: "not-forwardable", asked: true }),
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
    expect((lastSendParams as SendParams | undefined)?.forward_mode).toBe("not-forwardable");

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

describe("Group messages with a translation engine", () => {
  const nameOf = (addr: string) => addr;
  const row = (direction: "incoming" | "outgoing"): GroupMessageRow => ({
    id: `m-${direction}`,
    conversation: "grp-1",
    author: "did:key:zAlice",
    direction,
    sender_timestamp_ms: 1000,
    content_type: "text/plain",
    body_encoding: "utf8",
    body: "வணக்கம்",
    languages: ["ta"],
    state: "delivered",
  });

  afterEach(() => {
    vi.restoreAllMocks();
    setTargetsForTest([]);
  });

  it("an incoming message has a translate button, and the reader's own message does not", () => {
    readerReads([]);
    setTargetsForTest(["en"]);
    expect(renderMessageRow(row("incoming"), nameOf).querySelector(".message-translate")).not.toBeNull();
    expect(renderMessageRow(row("outgoing"), nameOf).querySelector(".message-translate")).toBeNull();
  });

  it("a node with no engine draws no translate button and makes no call", () => {
    readerReads([]);
    const calls = stubRpc({});
    expect(renderMessageRow(row("incoming"), nameOf).querySelector(".message-translate")).toBeNull();
    expect(calls).toEqual([]);
  });
});

describe("widget messages in groups thread", () => {
  const nameOf = (addr: string) => addr;
  const msg = (id: string, contentType: string, body?: string): GroupMessageRow => ({
    id,
    conversation: "grp-1",
    author: "did:key:zAlice",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    content_type: contentType,
    body_encoding: "utf8",
    body: body ?? "",
    state: "delivered",
  });

  it("a widget message draws the widget card in the group thread", () => {
    const threadList = document.createElement("div");
    const widgetView: WidgetView = {
      widget_id: "m-w1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      definition: {
        extension: { id: "poll", version: 1 },
        type: "poll/single",
        semantic: "poll",
        prompt: "Group lunch?",
        fields: [
          {
            name: "option",
            type: "choice",
            required: true,
            choices: [
              { code: "o1", text: "Tacos" },
              { code: "o2", text: "Salad" },
            ],
          },
        ],
      },
      checked: true,
      counts: [
        { field: "option", choice: "o1", count: 3 },
        { field: "option", choice: "o2", count: 0 },
      ],
      answers: [],
      ignored_answers: 0,
      state: "delivered",
    };
    const widgetMap = new Map<string, WidgetView>([["m-w1", widgetView]]);

    renderThreadMessages(
      threadList,
      [msg("m-w1", "application/vnd.roym.widget+json")],
      () => "Alice",
      () => {},
      () => {},
      widgetMap,
    );

    const card = threadList.querySelector(".widget-card");
    expect(card).not.toBeNull();
    expect(threadList.querySelector(".widget-card .message-body")?.textContent).toBe("Group lunch?");
  });

  it("an answer message in a group thread draws nothing (skipped)", () => {
    const threadList = document.createElement("div");
    renderThreadMessages(
      threadList,
      [msg("m-a1", "application/vnd.roym.widget-answer+json")],
      nameOf,
      () => {},
      () => {},
    );

    expect(threadList.children).toHaveLength(0);
  });

  it("a widget missing from the map keeps the widget prompt in the group thread", () => {
    const threadList = document.createElement("div");
    renderThreadMessages(
      threadList,
      [msg("m-w-missing", "application/vnd.roym.widget+json", JSON.stringify({
        widget_version: 1,
        widget: pollDefinition("Custom group prompt", ["A", "B"]),
      }))],
      nameOf,
      () => {},
      () => {},
      new Map(),
    );

    expect(threadList.querySelector(".widget-refused")).toBeNull();
    expect(threadList.textContent).toContain("Custom group prompt");
  });

  it("an own failed group widget row has exactly one state word and one Retry button", () => {
    const threadList = document.createElement("div");
    const m = msg("m-w-fail", "application/vnd.roym.widget+json", JSON.stringify({
      widget_version: 1,
      widget: pollDefinition("Group Question", ["A", "B"]),
    }));
    m.direction = "outgoing";
    m.state = "failed";

    renderThreadMessages(
      threadList,
      [m],
      nameOf,
      () => {},
      () => {},
      new Map(),
    );

    const statuses = threadList.querySelectorAll(".status-failed");
    expect(statuses).toHaveLength(1);

    const retries = threadList.querySelectorAll(".retry-button");
    expect(retries).toHaveLength(1);
  });

  it("a restored failed group widget row has exactly one state word and zero Retry buttons", () => {
    const threadList = document.createElement("div");
    const m = msg("m-w-res", "application/vnd.roym.widget+json", JSON.stringify({
      widget_version: 1,
      widget: pollDefinition("Group Question", ["A", "B"]),
    }));
    m.direction = "outgoing";
    m.state = "failed";
    m.restored = true;

    renderThreadMessages(
      threadList,
      [m],
      nameOf,
      () => {},
      () => {},
      new Map(),
    );

    const statuses = threadList.querySelectorAll(".status-failed");
    expect(statuses).toHaveLength(1);

    const retries = threadList.querySelectorAll(".retry-button");
    expect(retries).toHaveLength(0);
  });

  it("a group widget row shows the sender and has a delete button", () => {
    const threadList = document.createElement("div");
    const widgetView: WidgetView = {
      widget_id: "m-w1",
      author: "did:key:alice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      definition: {
        extension: { id: "poll", version: 1 },
        type: "poll/single",
        semantic: "poll",
        prompt: "Group lunch?",
        fields: [
          {
            name: "option",
            type: "choice",
            required: true,
            choices: [
              { code: "o1", text: "Pizza" },
            ],
          },
        ],
      },
      checked: true,
      counts: [],
      answers: [],
      ignored_answers: 0,
      state: "delivered",
    };
    const widgetMap = new Map<string, WidgetView>([["m-w1", widgetView]]);

    renderThreadMessages(
      threadList,
      [msg("m-w1", "application/vnd.roym.widget+json")],
      () => "Alice",
      () => {},
      () => {},
      widgetMap,
    );

    const row = threadList.querySelector(".message-row");
    expect(row).not.toBeNull();
    expect(row?.querySelector(".message-author")?.textContent).toBe("Alice");
    expect(row?.querySelector(".delete-button")).not.toBeNull();
  });

  it("reloadThread calls widget.list only for non-deleted widgets and skips when no widgets exist", async () => {
    const calls: { method: string; params: unknown }[] = [];
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 2,
            group: { name: "Team Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Team Group",
        owner_address: "did:key:zAlice",
        is_owner: true,
        is_member: true,
        restored_only: false,
        key_epoch: 1,
        key_stored_at_ms: 1000,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({
        messages: [
          {
            id: "m-w-live",
            conversation: "grp-1",
            author: "did:key:zAlice",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: '{"widget_version":1,"definition":{"prompt":"Live group poll"}}',
            state: "delivered",
            deleted_at_secs: null,
          },
          {
            id: "m-w-del",
            conversation: "grp-1",
            author: "did:key:zAlice",
            direction: "incoming",
            sender_timestamp_ms: 1100,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: "",
            state: "delivered",
            deleted_at_secs: 12345,
          },
        ],
      }),
      "widget.list": (params) => {
        calls.push({ method: "widget.list", params });
        return { widgets: [], complete: true };
      },
    });

    const host = document.createElement("div");
    await renderGroups(host);

    const widgetListCalls = calls.filter((c) => c.method === "widget.list");
    expect(widgetListCalls).toHaveLength(1);
    expect(widgetListCalls[0].params).toEqual({
      conversation: "grp-1",
      widget_ids: ["m-w-live"],
    });
  });

  it("on widget.list error in reloadThread, draws a thread-level error and keeps the widget's prompt", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: 1,
            group: { name: "Team Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        name: "Team Group",
        owner_address: "did:key:zAlice",
        is_owner: true,
        is_member: true,
        restored_only: false,
        key_epoch: 1,
        key_stored_at_ms: 1000,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.history": () => ({
        messages: [
          {
            id: "m-w-live",
            conversation: "grp-1",
            author: "did:key:zAlice",
            direction: "incoming",
            sender_timestamp_ms: 1000,
            content_type: "application/vnd.roym.widget+json",
            body_encoding: "utf8",
            body: JSON.stringify({
              widget_version: 1,
              widget: pollDefinition("Prompt on groups widget list failure", ["A", "B"]),
            }),
            state: "delivered",
          },
        ],
      }),
      "widget.list": () => {
        throw new Error("groups widget list error");
      },
    });

    const host = document.createElement("div");
    await renderGroups(host);

    const threadErr = host.querySelector(".widget-list-error");
    expect(threadErr).not.toBeNull();
    expect(threadErr?.textContent).toContain("groups widget list error");

    expect(host.querySelector(".widget-refused")).toBeNull();
    expect(host.textContent).toContain("Prompt on groups widget list failure");
  });
});

describe("Groups screen feed and forward work", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("EventSource", FakeFeedSource);
    _resetForTest();
  });

  afterEach(() => {
    document.body.replaceChildren();
    vi.useRealTimers();
  });

  function signedRow(id: string): GroupMessageRow {
    return {
      id,
      conversation: "grp-1",
      author: "did:key:zAlice",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      content_type: "application/vnd.roym.message+json",
      body_encoding: "utf8",
      body: "Hello",
      state: "delivered",
      forward: {
        mode: "signed",
        forwardable: true,
        origin: {
          author_did: "did:key:zAlice",
          written_at_secs: 1,
          record_id: "rec-1",
          is_you: false,
        },
        forward_count: 0,
        many_forwards: false,
        forwarded: false,
      },
    };
  }

  function stubGroupNode(messages: GroupMessageRow[], counters: { history: number; deleted: number }) {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          {
            id: "grp-1",
            kind: "group",
            message_count: messages.length,
            peer_address: "grp-1",
            group: { name: "Team Group", admission: { state: "shown" } },
          },
        ],
      }),
      "group.info": () => ({
        id: "grp-1",
        conversation_id: "grp-1",
        name: "Team Group",
        owner_address: "did:key:zAlice",
        is_owner: false,
        is_member: true,
        restored_only: false,
        epoch: 1,
        key_epoch: 1,
        key_stored_at_ms: 0,
        can_read_new_messages: true,
        members: [{ address: "did:key:zAlice", is_owner: true }],
      }),
      "conversation.transcript-digest": () => ({ digest: "abcdef123456" }),
      "contacts.list": () => [],
      "conversation.settings": () => ({ default_mode: "not-forwardable", asked: true }),
      "conversation.history": () => {
        counters.history++;
        return { messages };
      },
      "conversation.delete-message": () => {
        counters.deleted++;
        return {};
      },
    });
  }

  it("a feed event leaves an open forward picker", async () => {
    const counters = { history: 0, deleted: 0 };
    stubGroupNode([signedRow("msg-1")], counters);
    startFeed();
    const host = document.createElement("div");
    document.body.appendChild(host);
    await renderGroups(host);

    const forwardBtn = host.querySelector<HTMLButtonElement>(".forward-button")!;
    forwardBtn.click();
    await vi.advanceTimersByTimeAsync(0);
    const picker = host.querySelector(".forward-picker");
    expect(picker).not.toBeNull();
    const historyCalls = counters.history;

    FakeFeedSource.instance!.emit(
      "change",
      JSON.stringify({ kind: "conversation.message", subject: "grp-1", item: "msg-new" }),
      "cursor-new",
    );
    await vi.advanceTimersByTimeAsync(150);

    expect(host.querySelector(".forward-picker")).toBe(picker);
    expect(counters.history).toBe(historyCalls);

    host.querySelector<HTMLButtonElement>(".forward-picker-close")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(host.querySelector(".forward-picker")).toBeNull();
    expect(counters.history).toBe(historyCalls + 1);
  });

  it("a signed group message asks before deleting and says the origin stays", async () => {
    const counters = { history: 0, deleted: 0 };
    stubGroupNode([signedRow("msg-1")], counters);
    const host = document.createElement("div");
    document.body.appendChild(host);
    await renderGroups(host);

    host.querySelector<HTMLButtonElement>(".delete-button")!.click();
    await vi.advanceTimersByTimeAsync(0);

    const note = host.querySelector(".delete-signed-note");
    expect(note?.textContent).toBe(t("messages-delete-signed-note"));
    expect(counters.deleted).toBe(0);

    host.querySelector<HTMLButtonElement>(".confirm-delete")!.click();
    await vi.advanceTimersByTimeAsync(0);

    expect(counters.deleted).toBe(1);
    expect(host.querySelector(".delete-dialog")).toBeNull();
  });
});
