import { afterEach, describe, expect, it } from "vitest";
import { errText } from "../dom";
import { RpcError } from "../rpc";
import { activate, activateWith } from "./i18n";
import HINDI from "./hi.ftl?raw";

afterEach(async () => {
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

  it("has no detail for the -32013 refusal and for an HTTP failure", async () => {
    await hindi();
    expect(errText(rpc(-32013))).toMatch(DEVANAGARI);
    expect(errText(rpc(-32013))).not.toContain("raw text");
    expect(errText(new RpcError(503, "", "Other"))).toContain("HTTP 503");
    expect(errText(new RpcError(503, "", "Other"))).toMatch(DEVANAGARI);
  });
});
