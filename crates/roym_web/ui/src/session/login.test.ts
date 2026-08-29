import { describe, it, expect } from "vitest";
import { z32Encode } from "./login";

// Vectors produced by the Rust `z32` crate (the encoding the auth service
// verifies against). If these drift, a browser-signed challenge stops
// verifying.
describe("z32Encode", () => {
  it("matches the Rust z32 crate for an ASCII string", () => {
    expect(z32Encode(new TextEncoder().encode("hello world"))).toBe(
      "pb1sa5dxrb5s6hucco",
    );
  });

  it("matches the Rust z32 crate for a 64-byte buffer (signature length)", () => {
    const buf = new Uint8Array(64);
    for (let i = 0; i < 64; i++) buf[i] = (i * 7 + 3) & 0xff;
    expect(z32Encode(buf)).toBe(
      "ycfbngy9rasueq4njfefqzufpt3ziycet6mj5jfmskhhbt6q4zqq84zt9d9ocdewdctn1cbz83nwaw44cfwg67u7o1f3fgpyw6zmmxy",
    );
  });

  it("returns an empty string for empty input", () => {
    expect(z32Encode(new Uint8Array(0))).toBe("");
  });
});
