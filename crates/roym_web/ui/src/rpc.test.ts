import { describe, expect, it, vi } from "vitest";
import { errText } from "./dom";
import { call, RpcError } from "./rpc";

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

  it("keeps the service's own text for -32013 and words it from the string catalog", async () => {
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

    const caught = await call("test").catch((e: unknown) => e);
    expect(caught).toBeInstanceOf(RpcError);
    expect((caught as RpcError).type).toBe("NotLocal");
    expect((caught as RpcError).code).toBe(-32013);
    expect((caught as RpcError).message).toBe("this method is reachable only from inside this installation");
    expect(errText(caught)).toBe("this installation refused a request that did not come from you");
  });

  it("keeps the status of an HTTP failure as the code", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 503 }));
    const caught = await call("test").catch((e: unknown) => e);
    expect((caught as RpcError).code).toBe(503);
    expect(errText(caught)).toBe("HTTP error 503");
  });

  it("turns a fetch that rejects into an RpcError with code 0", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
    const caught = await call("test").catch((e: unknown) => e);
    expect(caught).toBeInstanceOf(RpcError);
    expect((caught as RpcError).code).toBe(0);
    expect(errText(caught)).toBe("Could not reach this installation: TypeError: Failed to fetch");
  });

  it("turns a reply that is not JSON into an RpcError with code 0", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => {
          throw new SyntaxError("Unexpected token <");
        },
      })
    );
    const caught = await call("test").catch((e: unknown) => e);
    expect((caught as RpcError).code).toBe(0);
  });
});
