import { errText } from "../dom";
import { HubError } from "../hub-error";
import { t } from "../i18n/i18n";
import { call, RpcError } from "../rpc";
import type { MembershipVerdict } from "./membership";

/// One merged search hit, as `directory.find` returns it. Every field the
/// directory could not vouch for is still present -- an absent field is
/// what a UI turns into a positive default, so the node fills all of them.
export interface MergedHit {
  listing_id: string;
  record_id: string;
  issuer: string;
  title: string;
  summary: string;
  categories: string[];
  conversation_address: string;
  status: string;
  verified: boolean;
  revocation_status: string;
  age_secs: number;
  /// One entry per directory that served this listing. `membership` is the
  /// verdict this node reached from that directory's own signed evidence;
  /// `null` only for a source whose reply carried none.
  sources: Array<{
    directory: string;
    record_id: string;
    received_at_secs: number;
    membership?: MembershipVerdict | null;
  }>;
  versions_differ: boolean;
}

export interface RefusedHit {
  listing_id: string;
  reason: string;
  sources: string[];
}

/// Why one source contributed nothing, as the client saw it. `not-started`
/// means this node refused to start the batch (e.g. proxy call-all busy)
/// -- deliberately not folded into `timed-out`, because this node being
/// busy is not that directory's fault.
export type SourceOutcomeKind =
  | "ok"
  | "not-started"
  | "timed-out"
  | "not-found"
  | "refused"
  | "unreadable";

export interface SourceOutcome {
  source: string;
  kind: SourceOutcomeKind;
  verified: number;
  refused: number;
  /// The directory said it had more matches for this query than it would
  /// return -- its own statement, distinct from the merged page cap.
  truncated: boolean;
  message?: string;
}

export interface FindResult {
  runId: string;
  outcomes: SourceOutcome[];
  hits: MergedHit[];
  hitsTruncated: boolean;
  refused: RefusedHit[];
  refusedTruncated: boolean;
}

interface FindReply {
  run_id: string;
  outcomes: Array<{
    source: string;
    verified: number;
    refused: number;
    truncated?: boolean;
    error?: { kind: string; code?: number; message?: string; reason?: string } | null;
  }>;
  hits: MergedHit[];
  hits_truncated: boolean;
  refused: RefusedHit[];
  refused_truncated: boolean;
}

export async function findListings(
  query: Record<string, unknown>,
): Promise<FindResult> {
  let reply: FindReply;
  try {
    reply = await call<FindReply>("directory.find", { query });
  } catch (err) {
    if (err instanceof RpcError && err.code === 503) {
      throw new HubError(t("directory-search-not-started", { button: t("directory-search-button") }));
    }
    throw new HubError(
      t("directory-search-failed", {
        detail: errText(err),
        button: t("directory-search-button"),
      }),
    );
  }

  const outcomes: SourceOutcome[] = (reply.outcomes ?? []).map((o) => {
    let kind: SourceOutcomeKind = "ok";
    let message: string | undefined;
    if (o.error) {
      kind = (o.error.kind as SourceOutcomeKind) || "unreadable";
      message = o.error.message ?? o.error.reason;
    }
    return {
      source: o.source,
      kind,
      verified: o.verified ?? 0,
      refused: o.refused ?? 0,
      truncated: o.truncated ?? false,
      message,
    };
  });

  return {
    runId: reply.run_id,
    outcomes,
    hits: reply.hits ?? [],
    hitsTruncated: reply.hits_truncated ?? false,
    refused: reply.refused ?? [],
    refusedTruncated: reply.refused_truncated ?? false,
  };
}

export async function runEnvelope(runId: string, recordId: string): Promise<string | null> {
  const res = await call<{ envelope?: string } | null>("directory.run-envelope", {
    run_id: runId,
    record_id: recordId,
  });
  return res?.envelope ?? null;
}
