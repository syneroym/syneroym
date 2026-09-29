// A SynOrg's membership verdict, mirrored from `roym_core::membership`
// (Rust is the source of truth; a Rust-side test reads this file and
// compares the two notice strings verbatim).

import { RpcError } from "../rpc";
import { ageWords, dateWords } from "./words";

export const NO_INSTANT_REMOVAL_NOTICE =
  "A group's decision reaches copies other people already hold only when they next check. Nobody can promise it is removed everywhere at once.";

export const WITHHELD_REVOCATION_NOTICE =
  "This shows every withdrawal the group has published. It cannot show one the group chose not to publish.";

export type ModerationScope = { kind: "membership" } | { kind: "listing"; listing_id: string };

export interface MembershipScope {
  categories: string[];
  area: unknown[];
}

/// The verdict *this installation* reached from a group's signed evidence.
/// Never a claim the directory made about itself.
export type MembershipVerdict =
  | { state: "none" }
  | {
      state: "valid";
      credential_record_id: string;
      issuer: string;
      synorg_name: string;
      scope: MembershipScope;
      expires_at_secs: number;
      revocations_checked_as_of_secs: number;
    }
  | { state: "expired"; credential_record_id: string; expires_at_secs: number }
  | { state: "out-of-scope"; credential_record_id: string; outside: string[] }
  | {
      state: "revoked";
      credential_record_id: string;
      revocation_record_id: string;
      revoked_at_secs: number;
      reason: string;
    }
  | {
      state: "suspended";
      decision_record_id: string;
      scope: ModerationScope;
      rule: string;
      reason: string;
      since_secs: number;
      until_secs?: number;
    }
  | { state: "refused"; reason: string }
  | { state: "unknown"; reason: string };

/// What the person is told about a reason this node could not check. The
/// raw reason strings are the node's; the sentences are the Hub's.
function unknownReasonWords(reason: string): string {
  switch (reason) {
    case "issuer-changed":
      return "the group this directory speaks for is not the one you first saw when you added it";
    case "issuer-not-pinned":
      return "this node has not yet recorded which group this directory speaks for";
    default:
      return reason;
  }
}

function withReason(sentence: string, reason: string): string {
  return reason ? `${sentence} Reason given: ${reason}` : sentence;
}

/// One line about one source's membership evidence, in words. A missing
/// verdict reads as unknown, never as a positive default, and no line
/// uses the word "verified": the Hub says what was checked and where.
export function membershipWords(
  v: MembershipVerdict | null | undefined,
  sourceLabel: string,
  nowSecs: number = Math.floor(Date.now() / 1000),
): string {
  if (!v) return `Membership: unknown (${sourceLabel} gave no answer this node could use)`;
  switch (v.state) {
    case "valid":
      return (
        `Member of ${v.synorg_name}, checked on your node. ` +
        `Group withdrawals checked ${ageWords(Math.max(0, nowSecs - v.revocations_checked_as_of_secs))}.`
      );
    case "none":
      return `No membership shown by ${sourceLabel}`;
    case "unknown":
      return `Membership: unknown (${unknownReasonWords(v.reason)})`;
    case "refused":
      return "Membership evidence did NOT check out -- treat as unknown";
    case "expired":
      return `Membership expired on ${dateWords(v.expires_at_secs)}`;
    case "out-of-scope":
      return `Membership does not cover this listing: ${v.outside.join(", ")}`;
    case "revoked":
      return withReason(
        `Membership revoked: the group withdrew it on ${dateWords(v.revoked_at_secs)}.`,
        v.reason,
      );
    case "suspended": {
      const what = v.scope.kind === "listing" ? "This listing is suspended" : "Membership suspended";
      const until = v.until_secs ? `until ${dateWords(v.until_secs)}` : "until the group lifts it";
      return withReason(
        `${what} by the group under its rule "${v.rule}", since ${dateWords(v.since_secs)}, ${until}.`,
        v.reason,
      );
    }
  }
}

/// Why a publish was refused, from the refusal's own `data.membership`.
/// The person publishing is asking a different question from a consumer
/// reading a result: not "who shows a membership" but "does this group
/// admit me".
export function refusalWords(v: MembershipVerdict | null | undefined): string {
  if (!v || v.state === "none") return "This group has no membership on record for you.";
  return membershipWords(v, "this group");
}

/// Why a publish was refused. A group that does not admit a listing says
/// why in a structured detail, which becomes a sentence about the person's
/// own membership; any other refusal (over the limit, a draft) keeps the
/// text `otherwise` carries.
export function publishRefusalWords(err: unknown, otherwise: string): string {
  if (err instanceof RpcError) {
    const data = err.data as { admission?: string; membership?: MembershipVerdict } | undefined;
    if (data?.admission === "not-admitted") {
      return `this group did not admit this listing. ${refusalWords(data.membership)}`;
    }
  }
  return otherwise;
}

/// The group a directory said it is when this node first added it. Never
/// re-recorded by a later reply, and never checked against anything
/// outside that first reply.
export function pinnedIssuerWords(issuer: string | null | undefined): string {
  if (!issuer) return "This node has not recorded which group this directory speaks for.";
  return `The group this directory said it is when you added it: ${issuer}`;
}

/// How old a held check is, from the time this node fetched it.
export function checkedWords(asOfSecs: number, nowSecs: number = Math.floor(Date.now() / 1000)): string {
  if (!asOfSecs) return "never checked";
  return `checked ${ageWords(Math.max(0, nowSecs - asOfSecs))}`;
}

/// What `directory.check-standing` answers. `refreshed: false` means the
/// verdict is not fresh: either the directory could not be reached (the
/// held copy is shown, with `error`), or it answered as a different group
/// than the one first pinned (`issuer-changed`, no copy involved).
export interface CheckStandingReply {
  verdict: MembershipVerdict;
  as_of_secs?: number;
  refreshed: boolean;
  error?: string;
}

export const ISSUER_CHANGED_CHECK_WORDS =
  "This directory now says it speaks for a different group than the one you first saw. Its answer was not used.";

/// The one `refreshed: false` reply where the directory *was* reached.
export function issuerChanged(res: CheckStandingReply): boolean {
  return !res.refreshed && res.verdict.state === "unknown" && res.verdict.reason === "issuer-changed";
}
