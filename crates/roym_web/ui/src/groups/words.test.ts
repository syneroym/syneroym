import { describe, expect, it } from "vitest";
import { t } from "../i18n/i18n";
import { deliveryWords, membershipEventWords, shortAddress } from "./words";

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

describe("group notices", () => {
  it("all eleven notice sentences are in the string catalog", () => {
    const notices = [
      t("group-owner-can-read-notice"),
      t("group-key-trust-notice"),
      t("group-delivery-notice"),
      t("group-join-boundary-notice"),
      t("group-removed-notice"),
      t("group-restored-notice"),
      t("group-add-unreachable"),
      t("group-hidden-notice"),
      t("group-transcript-check-notice"),
      t("group-cards-not-allowed"),
      t("group-session-ended-notice"),
    ];
    for (const n of notices) {
      expect(n.length).toBeGreaterThan(10);
      expect(n).not.toMatch(/^group-/);
    }
  });
});
