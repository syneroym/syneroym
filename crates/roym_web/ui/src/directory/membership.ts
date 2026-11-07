// A SynOrg's membership verdict, mirrored from `roym_core::membership`
// (Rust is the source of truth). The two notices the Hub shows with a verdict
// are the messages `membership-no-instant-removal` and
// `membership-withheld-revocation` in the string catalog; a Rust-side test
// reads `en.ftl` and compares them verbatim.

import { RpcError } from "../rpc";
import { formatAge, formatUtcDate } from "../i18n/format";
import { t } from "../i18n/i18n";

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
      return t("membership-reason-issuer-changed");
    case "issuer-not-pinned":
      return t("membership-reason-issuer-not-pinned");
    default:
      return reason;
  }
}

/// The arguments of a message that ends with an optional reason.
function reasonArgs(reason: string): { has_reason: string; reason: string } {
  return { has_reason: reason ? "yes" : "no", reason };
}

/// One line about one source's membership evidence, in words. A missing
/// verdict reads as unknown, never as a positive default, and no line
/// uses the word "verified": the Hub says what was checked and where.
export function membershipWords(
  v: MembershipVerdict | null | undefined,
  sourceLabel: string,
  nowSecs: number = Math.floor(Date.now() / 1000),
): string {
  if (!v) return t("membership-unknown-no-answer", { source: sourceLabel });
  switch (v.state) {
    case "valid":
      return t("membership-valid", {
        name: v.synorg_name,
        age: formatAge(Math.max(0, nowSecs - v.revocations_checked_as_of_secs)),
      });
    case "none":
      return t("membership-none", { source: sourceLabel });
    case "unknown":
      return t("membership-unknown", { reason: unknownReasonWords(v.reason) });
    case "refused":
      return t("membership-refused");
    case "expired":
      return t("membership-expired", { date: formatUtcDate(v.expires_at_secs) });
    case "out-of-scope":
      return t("membership-out-of-scope", { outside: v.outside.join(", ") });
    case "revoked":
      return t("membership-revoked", {
        date: formatUtcDate(v.revoked_at_secs),
        ...reasonArgs(v.reason),
      });
    case "suspended":
      return t("membership-suspended", {
        scope: v.scope.kind,
        rule: v.rule,
        since: formatUtcDate(v.since_secs),
        until: v.until_secs ? "date" : "lifted",
        date: v.until_secs ? formatUtcDate(v.until_secs) : "",
        ...reasonArgs(v.reason),
      });
  }
}

/// Why a publish was refused, from the refusal's own `data.membership`.
/// The person publishing is asking a different question from a searcher
/// reading a result: not "who shows a membership" but "does this group
/// admit me".
export function refusalWords(v: MembershipVerdict | null | undefined): string {
  if (!v || v.state === "none") return t("membership-refusal-none");
  return membershipWords(v, t("membership-this-group"));
}

/// Why a publish was refused. A group that does not admit a listing says
/// why in a structured detail, which becomes a sentence about the person's
/// own membership; any other refusal (over the limit, a draft) keeps the
/// text `otherwise` carries.
export function publishRefusalWords(err: unknown, otherwise: string): string {
  if (err instanceof RpcError) {
    const data = err.data as { admission?: string; membership?: MembershipVerdict } | undefined;
    if (data?.admission === "not-admitted") {
      return t("membership-publish-not-admitted", { why: refusalWords(data.membership) });
    }
  }
  return otherwise;
}

/// The group a directory said it is when this node first added it. Never
/// re-recorded by a later reply, and never checked against anything
/// outside that first reply.
export function pinnedIssuerWords(issuer: string | null | undefined): string {
  if (!issuer) return t("membership-issuer-unrecorded");
  return t("membership-issuer-pinned", { issuer });
}

/// How old a held check is, from the time this node fetched it.
export function checkedWords(asOfSecs: number, nowSecs: number = Math.floor(Date.now() / 1000)): string {
  if (!asOfSecs) return t("membership-never-checked");
  return t("membership-checked", { age: formatAge(Math.max(0, nowSecs - asOfSecs)) });
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

/// The one `refreshed: false` reply where the directory *was* reached.
export function issuerChanged(res: CheckStandingReply): boolean {
  return !res.refreshed && res.verdict.state === "unknown" && res.verdict.reason === "issuer-changed";
}
