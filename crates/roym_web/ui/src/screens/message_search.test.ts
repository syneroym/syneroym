import { describe, expect, it } from "vitest";
import { searchHits, snippetAround } from "./message_search.js";

describe("conversation.search matches to result-list hits", () => {
  // The shape `conversation.search` returns: full stored message rows.
  const row = (id: string, conversation: string, body?: string) => ({
    id,
    conversation,
    author: "did:key:someone",
    direction: "incoming",
    sender_timestamp_ms: 1000,
    content_type: "text/plain",
    body_encoding: "utf8",
    ...(body === undefined ? {} : { body }),
    state: "delivered",
    stored_at_secs: 1,
  });

  it("maps each row with a body to its conversation and a snippet", () => {
    const hits = searchHits([row("m1", "conv1", "the hedge needs trimming"), row("m2", "conv2", "hedge")], "hedge");
    expect(hits).toEqual([
      { conversation: "conv1", snippet: "the hedge needs trimming" },
      { conversation: "conv2", snippet: "hedge" },
    ]);
  });

  it("skips a deleted row, which has no body", () => {
    const hits = searchHits([row("m1", "conv1"), row("m2", "conv1", "kept hedge")], "hedge");
    expect(hits).toEqual([{ conversation: "conv1", snippet: "kept hedge" }]);
  });

  it("returns no hits for no matches", () => {
    expect(searchHits([], "hedge")).toEqual([]);
  });

  it("passes markup through unchanged; the caller shows it via textContent", () => {
    const body = "<b onclick=x>hedge</b>";
    const [hit] = searchHits([row("m1", "conv1", body)], "hedge");
    expect(hit.snippet).toBe(body);
  });
});

describe("snippetAround", () => {
  it("cuts a long body around the first match, case-insensitive, with ellipses", () => {
    const body = `${"a".repeat(100)}HEDGE${"b".repeat(200)}`;
    const s = snippetAround(body, "hedge");
    expect(s).toBe(`…${"a".repeat(30)}HEDGE${"b".repeat(90)}…`);
  });

  it("adds no ellipsis on a side that was not cut", () => {
    expect(snippetAround("hedge at the start", "hedge")).toBe("hedge at the start");
  });

  it("falls back to the start of the body when the query is not found", () => {
    const body = "x".repeat(200);
    expect(snippetAround(body, "hedge")).toBe(`${"x".repeat(95)}…`);
  });
});
