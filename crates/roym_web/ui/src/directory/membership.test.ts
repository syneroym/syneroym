import { describe, expect, it } from "vitest";
import { RpcError } from "../rpc";
import {
  checkedWords,
  membershipWords,
  pinnedIssuerWords,
  publishRefusalWords,
  refusalWords,
  type MembershipVerdict,
} from "./membership";

const NOW = 1_800_000_000;
const DAY = 86_400;

const VERDICTS: Record<string, MembershipVerdict> = {
  none: { state: "none" },
  valid: {
    state: "valid",
    credential_record_id: "rec_c",
    issuer: "did:key:zIssuer",
    synorg_name: "Cycling Guild",
    scope: { categories: ["cycling"], area: [] },
    expires_at_secs: NOW + 30 * DAY,
    revocations_checked_as_of_secs: NOW - 3 * 3600,
  },
  expired: { state: "expired", credential_record_id: "rec_c", expires_at_secs: NOW - DAY },
  outOfScope: { state: "out-of-scope", credential_record_id: "rec_c", outside: ["plumbing", "area"] },
  revoked: {
    state: "revoked",
    credential_record_id: "rec_c",
    revocation_record_id: "rec_r",
    revoked_at_secs: NOW - DAY,
    reason: "left the guild",
  },
  suspendedTimed: {
    state: "suspended",
    decision_record_id: "rec_d",
    scope: { kind: "membership" },
    rule: "r1",
    reason: "late twice",
    since_secs: NOW - 2 * DAY,
    until_secs: NOW + 5 * DAY,
  },
  suspendedListing: {
    state: "suspended",
    decision_record_id: "rec_d",
    scope: { kind: "listing", listing_id: "lst_x" },
    rule: "r2",
    reason: "",
    since_secs: NOW - DAY,
  },
  refused: { state: "refused", reason: "issuer mismatch" },
  unknown: { state: "unknown", reason: "issuer-changed" },
};

describe("membershipWords", () => {
  it("never uses the word verified, for any verdict or a missing one", () => {
    const all = [...Object.values(VERDICTS), null, undefined];
    for (const v of all) {
      expect(membershipWords(v, "Cycling Guild", NOW).toLowerCase()).not.toContain("verified");
    }
    expect(refusalWords(null).toLowerCase()).not.toContain("verified");
    expect(pinnedIssuerWords("did:key:zIssuer").toLowerCase()).not.toContain("verified");
  });

  it("says where a valid membership was checked and how old the group's withdrawals are", () => {
    expect(membershipWords(VERDICTS.valid, "src", NOW)).toBe(
      "Member of Cycling Guild, checked on your node. Group withdrawals checked 3 hours ago.",
    );
  });

  it("reads a missing verdict as unknown, not as a membership", () => {
    expect(membershipWords(undefined, "did:key:zSrc", NOW)).toMatch(/^Membership: unknown/);
    expect(membershipWords(null, "did:key:zSrc", NOW)).toContain("did:key:zSrc");
  });

  it("names the source when it shows no membership", () => {
    expect(membershipWords(VERDICTS.none, "Cycling Guild", NOW)).toBe(
      "No membership shown by Cycling Guild",
    );
  });

  it("spells out an unknown reason and a refused evidence check", () => {
    expect(membershipWords(VERDICTS.unknown, "src", NOW)).toContain(
      "not the one you first saw when you added it",
    );
    expect(membershipWords({ state: "unknown", reason: "issuer-not-pinned" }, "src", NOW)).toContain(
      "not yet recorded",
    );
    expect(membershipWords({ state: "unknown", reason: "something else" }, "src", NOW)).toContain(
      "something else",
    );
    expect(membershipWords(VERDICTS.refused, "src", NOW)).toBe(
      "Membership evidence did NOT check out -- treat as unknown",
    );
  });

  it("gives the date an expired membership ended and the categories one did not cover", () => {
    expect(membershipWords(VERDICTS.expired, "src", NOW)).toBe("Membership expired on 2027-01-14");
    expect(membershipWords(VERDICTS.outOfScope, "src", NOW)).toBe(
      "Membership does not cover this listing: plumbing, area",
    );
  });

  it("names the date and reason for a revocation, and omits an empty reason", () => {
    expect(membershipWords(VERDICTS.revoked, "src", NOW)).toBe(
      "Membership revoked: the group withdrew it on 2027-01-14. Reason given: left the guild",
    );
    const noReason = { ...VERDICTS.revoked, reason: "" } as MembershipVerdict;
    expect(membershipWords(noReason, "src", NOW)).not.toContain("Reason given");
  });

  it("names the rule, the start, and the end -- or that only the group can lift it", () => {
    expect(membershipWords(VERDICTS.suspendedTimed, "src", NOW)).toBe(
      'Membership suspended by the group under its rule "r1", since 2027-01-13, until 2027-01-20. ' +
        "Reason given: late twice",
    );
    expect(membershipWords(VERDICTS.suspendedListing, "src", NOW)).toBe(
      'This listing is suspended by the group under its rule "r2", since 2027-01-14, until the group lifts it.',
    );
  });
});

describe("refusalWords and issuer words", () => {
  it("tells a publisher with no membership that the group has none for them", () => {
    expect(refusalWords({ state: "none" })).toBe("This group has no membership on record for you.");
    expect(refusalWords(undefined)).toBe("This group has no membership on record for you.");
  });

  it("gives any other refusal the same words a consumer would read", () => {
    expect(refusalWords(VERDICTS.revoked)).toContain("Membership revoked");
  });

  it("words the pinned issuer as what the directory said, not as a fact", () => {
    expect(pinnedIssuerWords("did:key:zIssuer")).toBe(
      "The group this directory said it is when you added it: did:key:zIssuer",
    );
    expect(pinnedIssuerWords(null)).toContain("has not recorded");
  });

  it("says how old a check is, and that none was made when there is no date", () => {
    expect(checkedWords(NOW - 3 * 3600, NOW)).toBe("checked 3 hours ago");
    expect(checkedWords(0, NOW)).toBe("never checked");
  });
});

describe("publishRefusalWords", () => {
  it("turns a not-admitted refusal into a sentence about the person's own membership", () => {
    const err = new RpcError(-32602, "this SynOrg does not admit this listing: none", "Other", {
      admission: "not-admitted",
      membership: { state: "none" },
    });
    expect(publishRefusalWords(err, "raw")).toBe(
      "this group did not admit this listing. This group has no membership on record for you.",
    );
  });

  it("carries a revoked membership's own words", () => {
    const err = new RpcError(-32602, "x", "Other", {
      admission: "not-admitted",
      membership: VERDICTS.revoked,
    });
    expect(publishRefusalWords(err, "raw")).toContain("Membership revoked");
  });

  it("leaves every other refusal, and every other kind of error, as it was", () => {
    const overLimit = new RpcError(-32602, "over the limit", "Other", { retry_after_secs: 60 });
    expect(publishRefusalWords(overLimit, "over the limit")).toBe("over the limit");
    expect(publishRefusalWords(new Error("boom"), "boom")).toBe("boom");
  });
});
