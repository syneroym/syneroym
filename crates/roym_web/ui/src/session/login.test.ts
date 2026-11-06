import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { errText } from "../dom";
import { activate, activateWith } from "../i18n/i18n";
import HINDI from "../i18n/hi.ftl?raw";
import { leftoverLatin } from "../testing/latin-words";
import {
  clearEventFeedCookie,
  EVENT_FEED_COOKIE_MAX_AGE_SECS,
  loginWithBundle,
  setEventFeedCookie,
  z32Encode,
  type SessionKeyBundle,
} from "./login";

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

describe("event feed cookie", () => {
  const written: string[] = [];

  beforeEach(() => {
    Object.defineProperty(document, "cookie", {
      configurable: true,
      get: () => "",
      set: (value: string) => {
        written.push(value);
      },
    });
  });

  afterEach(() => {
    written.length = 0;
    Reflect.deleteProperty(document, "cookie");
  });

  it("is_scoped_to_the_feed_path_and_lives_one_hour", () => {
    setEventFeedCookie("abc123");
    expect(written).toEqual([
      `syneroym_session=abc123; path=/_syneroym/events; Max-Age=${EVENT_FEED_COOKIE_MAX_AGE_SECS}; SameSite=Lax`,
    ]);
    expect(EVENT_FEED_COOKIE_MAX_AGE_SECS).toBe(3600);
  });

  it("is_cleared_on_the_feed_path_and_on_the_old_root_path", () => {
    clearEventFeedCookie();
    expect(written).toEqual([
      "syneroym_session=; path=/_syneroym/events; Max-Age=0; SameSite=Lax",
      "syneroym_session=; path=/; Max-Age=0; SameSite=Lax",
    ]);
  });
});

describe("a failed login, as the person reads it", () => {
  const BUNDLE = {
    temp_secret_key_hex: "11".repeat(32),
    temp_did: "did:key:zTemp",
    certificate: { master_did: "did:key:zMaster", temporary_did: "did:key:zTemp" },
  } as unknown as SessionKeyBundle;

  /// The challenge step succeeds; the login step answers with `loginReply`.
  function fakeAuth(loginReply: { status: number; body?: unknown }): void {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url.endsWith("/challenge")) {
          return { ok: true, status: 200, json: async () => ({ nonce: "n", assertion: "a" }) };
        }
        return {
          ok: false,
          status: loginReply.status,
          json: async () => {
            if (loginReply.body === undefined) throw new SyntaxError("no body");
            return loginReply.body;
          },
        };
      }),
    );
  }

  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  async function failure(): Promise<string> {
    const err = await loginWithBundle(BUNDLE).then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(err).toBeDefined();
    return errText(err);
  }

  it("shows the status in English when the reply has no error text", async () => {
    fakeAuth({ status: 500 });
    expect(await failure()).toBe("HTTP 500");
  });

  it("shows the service text as it came in English", async () => {
    fakeAuth({ status: 401, body: { error: "key revoked" } });
    expect(await failure()).toBe("key revoked");
  });

  it("words the status in Hindi, with no English left", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    fakeAuth({ status: 500 });
    const text = await failure();
    expect(text).toMatch(/[ऀ-ॿ]/);
    expect(leftoverLatin(text)).toEqual([]);
  });

  it("puts the service text only after the detail label, once", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    fakeAuth({ status: 401, body: { error: "key revoked" } });
    const text = await failure();
    expect(text.match(/विवरण:/g)).toHaveLength(1);
    expect(text.endsWith("विवरण: key revoked")).toBe(true);
  });
});
