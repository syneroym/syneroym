import { describe, expect, it } from "vitest";
import { verifyParams } from "./listings.js";

describe("listings verifyParams", () => {
  it("never sends an object and sends trimmed text", () => {
    const raw = '  {"envelope": "test"}  \n';
    const params = verifyParams(raw);
    expect(typeof params.envelope).toBe("string");
    expect(params.envelope).toBe('{"envelope": "test"}');
  });
});
