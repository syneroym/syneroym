import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import { openForwardPicker } from "./forward_picker";

describe("the forward picker", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("lists the person's chats and forwards to the one picked", async () => {
    const calls: Array<[string, Record<string, unknown>]> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init: RequestInit) => {
        const body = JSON.parse(String(init.body)) as { method: string; params: Record<string, unknown> };
        calls.push([body.method, body.params]);
        return {
          ok: true,
          json: async () => ({
            result: {
              conversations: [
                { id: "conv-1", kind: "direct", peer_address: "did:key:peer-1" },
                { id: "conv-2", kind: "group", peer_address: "", name: "Market" },
              ],
            },
          }),
        };
      }),
    );

    const host = document.createElement("div");
    const picked: string[] = [];
    await openForwardPicker(host, async (conversationId) => {
      picked.push(conversationId);
    });

    expect(calls[0][0]).toBe("conversation.list");
    const rows = host.querySelectorAll<HTMLButtonElement>(".forward-picker-row");
    expect(Array.from(rows).map((r) => r.textContent)).toEqual(["did:key:peer-1", "Market"]);
    rows[1].click();
    await vi.waitFor(() => expect(picked).toEqual(["conv-2"]));
    expect(host.querySelector(".forward-picker")).toBeNull();
  });

  it("stays open and says why when the forward is refused", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init: RequestInit) => {
        const body = JSON.parse(String(init.body)) as { method: string };
        if (body.method === "conversation.list") {
          return {
            ok: true,
            json: async () => ({ result: { conversations: [{ id: "conv-1", peer_address: "p" }] } }),
          };
        }
        return { ok: true, json: async () => ({ error: { code: -32602, message: "refused" } }) };
      }),
    );

    const host = document.createElement("div");
    await openForwardPicker(host, async () => {
      throw new Error("not forwardable");
    });
    host.querySelector<HTMLButtonElement>(".forward-picker-row")!.click();

    await vi.waitFor(() =>
      expect(host.querySelector(".forward-picker-error")?.textContent).toContain(
        "Could not forward",
      ),
    );
    expect(host.querySelector(".forward-picker")).not.toBeNull();
  });

  it("says when there is no other chat", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: true, json: async () => ({ result: { conversations: [] } }) })),
    );
    const host = document.createElement("div");
    await openForwardPicker(host, async () => {});
    expect(host.querySelector(".forward-picker-empty")?.textContent).toBe(
      "There is no other chat to forward to.",
    );
  });
});
