/// One row of `conversation.search`'s `matches`: a stored message row. Only
/// the fields the result list reads are named here.
export interface SearchMatch {
  conversation: string;
  body?: string;
}

export interface SearchHit {
  conversation: string;
  snippet: string;
}

const BEFORE = 30;
const AFTER = 90;

/// A short piece of `body` around the first place `query` appears, with an
/// ellipsis on each side that was cut. The result is plain text and must
/// only ever be shown through `textContent`: the body is a stranger's bytes.
export function snippetAround(body: string, query: string): string {
  const at = Math.max(0, body.toLowerCase().indexOf(query.toLowerCase()));
  const start = Math.max(0, at - BEFORE);
  const end = Math.min(body.length, at + query.length + AFTER);
  const head = start > 0 ? "…" : "";
  const tail = end < body.length ? "…" : "";
  return head + body.slice(start, end) + tail;
}

/// Maps the service's `matches` to result-list entries. A deleted message
/// has no body; it has nothing to show, so it is skipped.
export function searchHits(matches: SearchMatch[], query: string): SearchHit[] {
  const hits: SearchHit[] = [];
  for (const m of matches) {
    if (!m.body) continue;
    hits.push({
      conversation: m.conversation,
      snippet: snippetAround(m.body, query),
    });
  }
  return hits;
}
