import { afterEach, describe, expect, it, vi } from "vitest";
import { errText } from "../dom";
import { HubError } from "../hub-error";
import { call, RpcError } from "../rpc";
import { stubRpc } from "../testing/stub-rpc";
import { activate, activateWith, t } from "./i18n";
import HINDI from "./hi.ftl?raw";

afterEach(async () => {
  vi.unstubAllGlobals();
  await activate("en");
});

const DEVANAGARI = /^[ऀ-ॿ]/;
const NAMED_CODES = [
  -32001, -32002, -32003, -32004, -32010, -32011, -32012, -32601, -32602, -32603,
];
const OTHER_CODES = [-32700, -32600, -32099, -12345];

function rpc(code: number, message = "raw text"): RpcError {
  return new RpcError(code, message, "Other");
}

describe("errText in English", () => {
  it("shows the service's own text for a named code", () => {
    for (const code of NAMED_CODES) expect(errText(rpc(code))).toBe("raw text");
  });

  it("shows the service's own text for any other code", () => {
    for (const code of OTHER_CODES) expect(errText(rpc(code))).toBe("raw text");
  });

  it("words the -32013 refusal itself", () => {
    expect(errText(rpc(-32013))).toBe("this installation refused a request that did not come from you");
  });

  it("words an HTTP failure from its status, without digit grouping", () => {
    expect(errText(new RpcError(503, "", "Other"))).toBe("HTTP error 503");
    expect(errText(new RpcError(1503, "", "Other"))).toBe("HTTP error 1503");
  });

  it("words a failure to reach the node", () => {
    expect(errText(rpc(0, "TypeError: Failed to fetch"))).toBe(
      "Could not reach this installation: TypeError: Failed to fetch",
    );
  });

  it("passes other errors and values through", () => {
    expect(errText(new Error("x"))).toBe("x");
    expect(errText("y")).toBe("y");
  });
});

/// The message each code must give. Written out here on purpose, so a swapped
/// or missing row in the table is seen.
const EXPECTED: Array<[number, string]> = [
  [-32001, "error-service-unavailable"],
  [-32002, "error-service-timeout"],
  [-32003, "error-permission"],
  [-32004, "error-quota"],
  [-32010, "error-not-signed-in"],
  [-32011, "error-not-owner"],
  [-32012, "error-no-owner"],
  [-32013, "error-not-local"],
  [-32601, "error-method-not-found"],
  [-32602, "error-invalid-input"],
  [-32603, "error-internal"],
  [-32700, "error-service"],
  [-32600, "error-service"],
  [-32099, "error-service"],
  [-12345, "error-service"],
];

describe("errText in Hindi", () => {
  async function hindi(): Promise<void> {
    await activateWith("hi", { hi: async () => HINDI });
  }

  it("starts with Hindi and puts the service text only after the detail label", async () => {
    await hindi();
    for (const code of [...NAMED_CODES, ...OTHER_CODES, 0]) {
      const text = errText(rpc(code));
      expect(text, String(code)).toMatch(DEVANAGARI);
      expect(text.replace(/विवरण: .*$/, ""), String(code)).not.toContain("raw text");
      expect(text, String(code)).toContain("विवरण: raw text");
    }
  });

  it("gives each code its own message", async () => {
    await hindi();
    for (const [code, id] of EXPECTED) {
      expect(errText(rpc(code)), String(code)).toBe(t(id, { detail: "raw text", code: String(code) }));
    }
    const distinct = new Set(EXPECTED.slice(0, 11).map(([code]) => errText(rpc(code))));
    expect(distinct.size).toBe(11);
  });

  it("has no detail for the -32013 refusal and for an HTTP failure", async () => {
    await hindi();
    expect(errText(rpc(-32013))).toMatch(DEVANAGARI);
    expect(errText(rpc(-32013))).not.toContain("raw text");
    expect(errText(new RpcError(503, "", "Other"))).toContain("HTTP 503");
    expect(errText(new RpcError(503, "", "Other"))).toMatch(DEVANAGARI);
  });
});

describe("errText for failures the Hub makes or the browser throws", () => {
  async function hindi(): Promise<void> {
    await activateWith("hi", { hi: async () => HINDI });
  }

  it("shows a HubError as it is, in both languages", async () => {
    expect(errText(new HubError("x"))).toBe("x");
    await hindi();
    expect(errText(new HubError("x"))).toBe("x");
  });

  it("puts a browser error only after the detail label in Hindi", async () => {
    expect(errText(new SyntaxError("Unexpected token"))).toBe("Unexpected token");
    await hindi();
    const text = errText(new SyntaxError("Unexpected token"));
    expect(text).toMatch(DEVANAGARI);
    expect(text.replace(/विवरण: .*$/, "")).not.toContain("Unexpected");
    expect(text).toContain("विवरण: Unexpected token");
  });

  it("words a thrown string like a browser error", async () => {
    expect(errText("oops")).toBe("oops");
    await hindi();
    const text = errText("oops");
    expect(text).toMatch(DEVANAGARI);
    expect(text).toContain("विवरण: oops");
  });

  it("gives an RpcError with no message a detail from the string catalog", async () => {
    stubRpc({ "x.y": () => new RpcError(-32602, "", "Other") });
    const err = await call("x.y").catch((e: unknown) => e);
    expect(errText(err)).toBe("RPC error");
    await hindi();
    expect(errText(err)).toContain("विवरण: RPC error");
  });
});
