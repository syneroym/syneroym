import { describe, expect, it } from "vitest";
import { upsertParams } from "./contacts.js";

describe("contacts upsertParams", () => {
  it("sends text as the same string and never sends a parsed object", () => {
    const rawEnvelope = '{"record":"profile","display_name":"Alice"}';
    const params = upsertParams("did:key:zAlice", "addr-123", rawEnvelope);
    expect(typeof params.profile_envelope).toBe("string");
    expect(params.profile_envelope).toBe(rawEnvelope);
  });

  it("does not send profile_envelope when empty or whitespace-only", () => {
    const emptyParams = upsertParams("did:key:zAlice", "addr-123", "");
    expect("profile_envelope" in emptyParams).toBe(false);

    const wsParams = upsertParams("did:key:zAlice", "addr-123", "   \n\t  ");
    expect("profile_envelope" in wsParams).toBe(false);
  });

  it("trims address and omits it when empty or whitespace-only", () => {
    const withAddr = upsertParams("  did:key:zAlice  ", "  addr-456  ", "");
    expect(withAddr.person_did).toBe("did:key:zAlice");
    expect(withAddr.conversation_address).toBe("addr-456");

    const noAddr = upsertParams("did:key:zAlice", "", "");
    expect("conversation_address" in noAddr).toBe(false);

    const wsAddr = upsertParams("did:key:zAlice", "   ", "");
    expect("conversation_address" in wsAddr).toBe(false);
  });
});
