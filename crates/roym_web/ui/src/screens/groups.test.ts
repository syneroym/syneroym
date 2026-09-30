import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CARDS_NOT_IN_GROUPS_MESSAGE,
  GROUP_REMOVED_NOTICE,
  GROUP_RESTORED_NOTICE,
  MEMBERSHIP_EVENT_CONTENT_TYPE,
  SESSION_ENDED_NOTICE,
} from "../groups/words";
import { RpcError } from "../rpc";
import {
  renderGroups,
  renderMessageRow,
  renderThreadMessages,
  type GroupMessageRow,
} from "./groups";

type Handler = (params: Record<string, unknown>) => unknown;

function stubRpc(handlers: Record<string, Handler>) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as {
        method: string;
        params: Record<string, unknown>;
      };
      const handler = handlers[body.method];
      if (!handler) throw new Error(`unexpected method ${body.method}`);
      const res = handler(body.params);
      if (res instanceof RpcError) {
        return {
          ok: true,
          json: async () => ({
            error: { code: res.code, message: res.message, data: res.data },
          }),
        };
      }
      return { ok: true, json: async () => ({ result: res }) };
    }),
  );
}

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
    expect(cards[0].textContent).toContain(CARDS_NOT_IN_GROUPS_MESSAGE);

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

    expect(host.textContent).toContain(SESSION_ENDED_NOTICE);
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

    expect(host.textContent).toContain(GROUP_RESTORED_NOTICE);
    expect(host.textContent).not.toContain(GROUP_REMOVED_NOTICE);
    expect(host.querySelector(".key-changed-date")).toBeNull();
  });
});
