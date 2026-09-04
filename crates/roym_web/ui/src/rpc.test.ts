import { describe, expect, it, vi } from "vitest";
import { call } from "./rpc";

describe("rpc call", () => {
  it("maps -32010 to NotSignedIn", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ jsonrpc: "2.0", error: { code: -32010, message: "not signed in" } }),
      })
    );

    try {
      await call("test");
    } catch (e: unknown) {
      expect((e as { type?: string }).type).toBe("NotSignedIn");
    }
  });

  it("maps -32011 to NotOwner", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ jsonrpc: "2.0", error: { code: -32011, message: "not owner" } }),
      })
    );

    try {
      await call("test");
    } catch (e: unknown) {
      expect((e as { type?: string }).type).toBe("NotOwner");
    }
  });

  it("maps -32012 to NoOwner", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ jsonrpc: "2.0", error: { code: -32012, message: "no owner" } }),
      })
    );

    try {
      await call("test");
    } catch (e: unknown) {
      expect((e as { type?: string }).type).toBe("NoOwner");
    }
  });

  it("maps -32013 to NotLocal with the fixed installation-refused message", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          jsonrpc: "2.0",
          error: { code: -32013, message: "this method is reachable only from inside this installation" },
        }),
      })
    );

    let caught: { type?: string; message?: string; code?: number } | undefined;
    try {
      await call("test");
    } catch (e: unknown) {
      caught = e as { type?: string; message?: string; code?: number };
    }
    expect(caught?.type).toBe("NotLocal");
    expect(caught?.code).toBe(-32013);
    expect(caught?.message).toBe("this installation refused a request that did not come from you");
  });
});
