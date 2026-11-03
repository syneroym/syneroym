import { afterEach, describe, expect, it, vi } from "vitest";
import { findListings } from "./search";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("findListings", () => {
  it("findListings_maps_each_error_kind_to_an_outcome", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        json: async () => ({
          result: {
            run_id: "run_test_1",
            outcomes: [
              { source: "did:key:s1", verified: 2, refused: 0, error: null },
              { source: "did:key:s2", verified: 0, refused: 0, error: { kind: "not-started" } },
              { source: "did:key:s3", verified: 0, refused: 0, error: { kind: "timed-out" } },
              { source: "did:key:s4", verified: 0, refused: 0, error: { kind: "not-found" } },
              {
                source: "did:key:s5",
                verified: 0,
                refused: 1,
                error: { kind: "refused", code: -32600, message: "bad request" },
              },
              {
                source: "did:key:s6",
                verified: 0,
                refused: 0,
                error: { kind: "unreadable", reason: "reply too large" },
              },
            ],
            hits: [],
            hits_truncated: false,
            refused: [],
            refused_truncated: false,
          },
        }),
      })),
    );

    const res = await findListings({});
    expect(res.outcomes).toHaveLength(6);
    expect(res.outcomes[0]).toEqual({
      source: "did:key:s1",
      kind: "ok",
      verified: 2,
      refused: 0,
      truncated: false,
      message: undefined,
    });
    expect(res.outcomes[1]?.kind).toBe("not-started");
    expect(res.outcomes[2]?.kind).toBe("timed-out");
    expect(res.outcomes[3]?.kind).toBe("not-found");
    expect(res.outcomes[4]).toEqual({
      source: "did:key:s5",
      kind: "refused",
      verified: 0,
      refused: 1,
      truncated: false,
      message: "bad request",
    });
    expect(res.outcomes[5]).toEqual({
      source: "did:key:s6",
      kind: "unreadable",
      verified: 0,
      refused: 0,
      truncated: false,
      message: "reply too large",
    });
  });

  it("findListings_turns_an_http_503_into_a_busy_notice", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: false,
        status: 503,
        json: async () => ({}),
      })),
    );

    await expect(findListings({})).rejects.toThrow(
      "This installation could not start the search. Press Search to try again.",
    );
  });

  it("findListings_turns_any_other_failure_into_a_page_error", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        json: async () => ({
          error: {
            code: -32602,
            message: "invalid params: query.open_to: unknown variant",
          },
        }),
      })),
    );

    await expect(findListings({})).rejects.toThrow(
      "The search failed: invalid params: query.open_to: unknown variant. Press Search to try again.",
    );
  });

  it("findListings_with_no_sources_returns_no_outcomes", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        json: async () => ({
          result: {
            run_id: "run_empty",
            outcomes: [],
            hits: [],
            hits_truncated: false,
            refused: [],
            refused_truncated: false,
          },
        }),
      })),
    );

    const res = await findListings({});
    expect(res.outcomes).toEqual([]);
    expect(res.hits).toEqual([]);
    expect(res.runId).toBe("run_empty");
  });

  it("findListings_sends_one_request", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        result: {
          run_id: "run_single",
          outcomes: [],
          hits: [],
          hits_truncated: false,
          refused: [],
          refused_truncated: false,
        },
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await findListings({ text: "cycling" });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const callArgs = fetchMock.mock.calls[0] as [string, RequestInit];
    const body = JSON.parse(String(callArgs[1].body)) as {
      method: string;
      params: { query: { text: string } };
    };
    expect(body.method).toBe("directory.find");
    expect(body.params.query.text).toBe("cycling");
  });
});
