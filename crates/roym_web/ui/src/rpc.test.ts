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
});
