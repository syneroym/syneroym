import { call, RpcError } from "../rpc";

/// One merged search hit, as `directory.merge` returns it. Every field the
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
  credential: string;
  age_secs: number;
  sources: Array<{ directory: string; record_id: string; received_at_secs: number }>;
  versions_differ: boolean;
}

export interface RefusedHit {
  listing_id: string;
  reason: string;
  sources: string[];
}

export interface MergeResult {
  hits: MergedHit[];
  hits_truncated: boolean;
  refused: RefusedHit[];
  refused_truncated: boolean;
}

/// Why one source contributed nothing, as the client saw it. `NotStarted`
/// is this installation's own gateway refusing to begin the call (a 503
/// from guest-HTTP admission) -- deliberately not folded into `TimedOut`,
/// because this node being busy is not that directory's fault.
export type SourceOutcomeKind = "ok" | "not-started" | "timed-out" | "not-found" | "refused" | "unreadable";

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

export interface StartedRun {
  runId: string;
  sources: string[];
  maxConcurrency: number;
}

export async function startRun(): Promise<StartedRun> {
  const res = await call<{ run_id: string; sources: string[]; max_concurrency: number }>(
    "directory.start-run",
  );
  return {
    runId: res.run_id,
    sources: res.sources ?? [],
    // The node owns this number because it owns the admission limit it is
    // derived from; the client never picks its own.
    maxConcurrency: Math.max(1, res.max_concurrency ?? 1),
  };
}

interface QuerySourceReply {
  source: string;
  verified: number;
  refused: number;
  truncated?: boolean;
  error: { kind: string; code?: number; message?: string } | null;
}

/// Ask one directory. A 503 from this node's own gateway is `not-started`;
/// every other failure arrives inside the reply's `error`.
async function queryOneSource(
  runId: string,
  source: string,
  query: Record<string, unknown>,
): Promise<SourceOutcome> {
  try {
    const reply = await call<QuerySourceReply>("directory.query-source", {
      run_id: runId,
      source,
      query,
    });
    if (reply.error) {
      return {
        source,
        kind: (reply.error.kind as SourceOutcomeKind) || "unreadable",
        verified: reply.verified ?? 0,
        refused: reply.refused ?? 0,
        truncated: reply.truncated ?? false,
        message: reply.error.message,
      };
    }
    return {
      source,
      kind: "ok",
      verified: reply.verified ?? 0,
      refused: reply.refused ?? 0,
      truncated: reply.truncated ?? false,
    };
  } catch (err) {
    if (err instanceof RpcError && err.code === 503) {
      return { source, kind: "not-started", verified: 0, refused: 0, truncated: false };
    }
    return {
      source,
      kind: "unreadable",
      verified: 0,
      refused: 0,
      truncated: false,
      message: err instanceof Error ? err.message : String(err),
    };
  }
}

/// The client's fan-out loop, the one place it lives in the Hub. Runs
/// `query-source` for every source, at most `maxConcurrency` in flight,
/// calling `onSource` as each one answers so the view can render
/// progressively, then returns `directory.merge`'s result.
export interface SearchProgress {
  answered: number;
  total: number;
  runId: string;
  /// `"fanout"` while the run is still answering sources; `"retry"` for
  /// the second attempt at a source this node was too busy to start. In
  /// the retry phase `answered` is already `total` (every source
  /// answered once), so the view should not render it as a live count.
  phase: "fanout" | "retry";
}

export interface RunSearchOptions {
  onSource?: (outcome: SourceOutcome, progress: SearchProgress) => void | Promise<void>;
  /// Only the browser test suite passes this: it oversubscribes this
  /// node's own guest-HTTP admission limit on purpose, to prove a 503 from
  /// the node becomes `not-started` and never `timed-out`. The Hub itself
  /// always honours `max_concurrency`.
  ignoreConcurrency?: boolean;
}

export async function runSearch(
  query: Record<string, unknown>,
  onSourceOrOpts?:
    | ((outcome: SourceOutcome, progress: SearchProgress) => void | Promise<void>)
    | RunSearchOptions,
): Promise<{ run: StartedRun; outcomes: SourceOutcome[]; merged: MergeResult }> {
  const opts: RunSearchOptions =
    typeof onSourceOrOpts === "function" ? { onSource: onSourceOrOpts } : onSourceOrOpts ?? {};
  const onSource = opts.onSource;
  const run = await startRun();
  const outcomes: SourceOutcome[] = [];
  const queue = [...run.sources];
  let answered = 0;

  async function worker() {
    for (;;) {
      const source = queue.shift();
      if (source === undefined) return;
      const outcome = await queryOneSource(run.runId, source, query);
      outcomes.push(outcome);
      answered += 1;
      await onSource?.(outcome, {
        answered,
        total: run.sources.length,
        runId: run.runId,
        phase: "fanout",
      });
    }
  }

  const concurrency = opts.ignoreConcurrency
    ? run.sources.length
    : Math.min(run.maxConcurrency, run.sources.length);
  await Promise.all(Array.from({ length: concurrency }, worker));

  // A source this node refused to start may be retried once the run's
  // other calls have drained -- the admission permits are free again.
  const retryable = opts.ignoreConcurrency
    ? []
    : outcomes.filter((o) => o.kind === "not-started").map((o) => o.source);
  for (const source of retryable) {
    const again = await queryOneSource(run.runId, source, query);
    const idx = outcomes.findIndex((o) => o.source === source);
    if (idx >= 0) outcomes[idx] = again;
    await onSource?.(again, {
      answered,
      total: run.sources.length,
      runId: run.runId,
      phase: "retry",
    });
  }

  const merged = await call<MergeResult>("directory.merge", { run_id: run.runId });
  return { run, outcomes, merged };
}

export async function runEnvelope(runId: string, recordId: string): Promise<string | null> {
  const res = await call<{ envelope?: string } | null>("directory.run-envelope", {
    run_id: runId,
    record_id: recordId,
  });
  return res?.envelope ?? null;
}
