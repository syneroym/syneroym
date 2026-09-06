import { afterEach, describe, expect, it, vi } from "vitest";
import { runSearch } from "./search";

/// A fake `fetch` that answers the three directory verbs. `sources` is the
/// list `start-run` hands back; `reply` decides what each `query-source`
/// call returns (an object -> ok reply; the string "503" -> an HTTP 503).
function stubRpc(opts: {
  sources: string[];
  maxConcurrency: number;
  reply: (source: string, callNo: number) => Record<string, unknown> | "503";
  onInFlight?: (n: number) => void;
}) {
  const callCounts: Record<string, number> = {};
  let inFlight = 0;
  let maxObservedInFlight = 0;

  const fetchImpl = vi.fn(async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as { method: string; params: Record<string, unknown> };
    if (body.method === "directory.start-run") {
      return {
        ok: true,
        json: async () => ({
          result: {
            run_id: "run_test_0",
            sources: opts.sources,
            max_concurrency: opts.maxConcurrency,
          },
        }),
      };
    }
    if (body.method === "directory.query-source") {
      inFlight += 1;
      maxObservedInFlight = Math.max(maxObservedInFlight, inFlight);
      opts.onInFlight?.(inFlight);
      await new Promise((r) => setTimeout(r, 5));
      const source = body.params.source as string;
      callCounts[source] = (callCounts[source] ?? 0) + 1;
      const out = opts.reply(source, callCounts[source]);
      inFlight -= 1;
      if (out === "503") {
        return { ok: false, status: 503, json: async () => ({}) };
      }
      return { ok: true, json: async () => ({ result: out }) };
    }
    if (body.method === "directory.merge") {
      return {
        ok: true,
        json: async () => ({
          result: { hits: [], hits_truncated: false, refused: [], refused_truncated: false },
        }),
      };
    }
    throw new Error(`unexpected method ${body.method}`);
  });

  vi.stubGlobal("fetch", fetchImpl);
  return {
    get maxObservedInFlight() {
      return maxObservedInFlight;
    },
    callCounts,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("directory client fan-out loop", () => {
  it("never runs more than max_concurrency query-source calls at once", async () => {
    const probe = stubRpc({
      sources: ["did:a", "did:b", "did:c", "did:d", "did:e"],
      maxConcurrency: 2,
      reply: () => ({ source: "x", verified: 1, refused: 0, error: null }),
    });
    await runSearch({});
    expect(probe.maxObservedInFlight).toBeLessThanOrEqual(2);
  });

  it("marks a 503 from this node as not-started, distinct from a directory timeout", async () => {
    stubRpc({
      sources: ["did:busy", "did:slow"],
      maxConcurrency: 3,
      reply: (source, callNo) => {
        if (source === "did:busy" && callNo === 1) return "503";
        if (source === "did:busy") return { source, verified: 0, refused: 0, error: null };
        return {
          source,
          verified: 0,
          refused: 0,
          error: { kind: "timed-out", message: "no answer" },
        };
      },
    });
    const { outcomes } = await runSearch({});
    const busy = outcomes.find((o) => o.source === "did:busy");
    const slow = outcomes.find((o) => o.source === "did:slow");
    // The 503 source is retried once the run drains, and the retry succeeded.
    expect(busy?.kind).toBe("ok");
    expect(slow?.kind).toBe("timed-out");
  });

  it("reports every source through the progress callback, exactly once each", async () => {
    const seen: string[] = [];
    stubRpc({
      sources: ["did:a", "did:b", "did:c"],
      maxConcurrency: 2,
      reply: (source) => ({ source, verified: 0, refused: 0, error: null }),
    });
    await runSearch({}, (outcome) => {
      seen.push(outcome.source);
    });
    expect(seen.sort()).toEqual(["did:a", "did:b", "did:c"]);
  });

  it("carries a directory's own truncated flag onto the source outcome", async () => {
    stubRpc({
      sources: ["did:full", "did:normal"],
      maxConcurrency: 3,
      reply: (source) => ({
        source,
        verified: 1,
        refused: 0,
        truncated: source === "did:full",
        error: null,
      }),
    });
    const { outcomes } = await runSearch({});
    expect(outcomes.find((o) => o.source === "did:full")?.truncated).toBe(true);
    expect(outcomes.find((o) => o.source === "did:normal")?.truncated).toBe(false);
  });

  it("a run with zero sources makes no query-source call and still merges", async () => {
    const probe = stubRpc({
      sources: [],
      maxConcurrency: 3,
      reply: () => ({ source: "x", verified: 0, refused: 0, error: null }),
    });
    const { merged, outcomes } = await runSearch({});
    expect(outcomes).toEqual([]);
    expect(Object.keys(probe.callCounts)).toEqual([]);
    expect(merged.hits).toEqual([]);
  });
});
