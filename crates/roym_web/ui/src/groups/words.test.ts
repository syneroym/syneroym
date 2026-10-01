import { describe, expect, it } from "vitest";
import {
  CARDS_NOT_IN_GROUPS_MESSAGE,
  GROUP_ADD_UNREACHABLE_MESSAGE,
  GROUP_DELIVERY_NOTICE,
  GROUP_HIDDEN_NOTICE,
  GROUP_JOIN_BOUNDARY_NOTICE,
  GROUP_KEY_TRUST_NOTICE,
  GROUP_REMOVED_NOTICE,
  GROUP_RESTORED_NOTICE,
  OWNER_CAN_READ_NOTICE,
  SESSION_ENDED_NOTICE,
  TRANSCRIPT_CHECK_NOTICE,
  deliveryWords,
  membershipEventWords,
  shortAddress,
} from "./words";

describe("deliveryWords", () => {
  it("maps pending, delivered, and failed states to exact words", () => {
    expect(deliveryWords("pending")).toBe("Not yet delivered to every member");
    expect(deliveryWords("delivered")).toBe("Delivered to every member");
    expect(deliveryWords("failed")).toBe("Not delivered to every member");
  });

  it("never contains forbidden words (verified, read, trying, or member names/addresses)", () => {
    const states = ["pending", "delivered", "failed"] as const;
    const forbidden = ["verified", "read", "trying", "alice", "bob", "did:key"];

    for (const state of states) {
      const words = deliveryWords(state).toLowerCase();
      for (const f of forbidden) {
        expect(words).not.toContain(f);
      }
    }
  });
});

describe("shortAddress", () => {
  it("leaves short addresses unchanged and abbreviates long DIDs", () => {
    expect(shortAddress("")).toBe("");
    expect(shortAddress("short-address")).toBe("short-address");
    expect(shortAddress("did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK")).toBe(
      "did:key:z6Mk…",
    );
  });
});

describe("membershipEventWords", () => {
  const names: Record<string, string> = {
    "did:key:zOwner": "Alice",
    "did:key:zMember1": "Bob",
    "did:key:zMember2": "Charlie",
  };
  const nameOf = (addr: string) => names[addr] || shortAddress(addr);

  it("formats creation events with display names", () => {
    const ev = { action: "create", subject: "did:key:zOwner", epoch: 1 };
    expect(membershipEventWords(ev, nameOf, "did:key:zOwner")).toBe("Alice created the group");
  });

  it("formats genesis add events as creation", () => {
    const ev = { action: "add", subject: "did:key:zOwner", epoch: 1 };
    expect(membershipEventWords(ev, nameOf, "did:key:zOwner")).toBe("Alice created the group");
  });

  it("formats member add events with display names", () => {
    const ev = { action: "add", subject: "did:key:zMember1", epoch: 2 };
    expect(membershipEventWords(ev, nameOf, "did:key:zOwner")).toBe("Alice added Bob");
  });

  it("formats member remove events with display names", () => {
    const ev = { action: "remove", subject: "did:key:zMember2", epoch: 3 };
    expect(membershipEventWords(ev, nameOf, "did:key:zOwner")).toBe("Alice removed Charlie");
  });

  it("falls back to short address when display names are unknown", () => {
    const unknownOwner = "did:key:zUnknownOwnerLongAddress123";
    const unknownSubject = "did:key:zUnknownSubjectLongAddress456";

    const addEv = { action: "add", subject: unknownSubject, epoch: 2 };
    expect(membershipEventWords(addEv, nameOf, unknownOwner)).toBe(
      "did:key:zUnk… added did:key:zUnk…",
    );
  });
});

describe("notice constants", () => {
  it("all nine notice strings are non-empty", () => {
    const notices = [
      OWNER_CAN_READ_NOTICE,
      GROUP_KEY_TRUST_NOTICE,
      GROUP_DELIVERY_NOTICE,
      GROUP_JOIN_BOUNDARY_NOTICE,
      GROUP_REMOVED_NOTICE,
      GROUP_RESTORED_NOTICE,
      GROUP_ADD_UNREACHABLE_MESSAGE,
      GROUP_HIDDEN_NOTICE,
      TRANSCRIPT_CHECK_NOTICE,
      CARDS_NOT_IN_GROUPS_MESSAGE,
      SESSION_ENDED_NOTICE,
    ];
    for (const n of notices) {
      expect(typeof n).toBe("string");
      expect(n.length).toBeGreaterThan(10);
    }
  });
});
