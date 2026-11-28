import { afterEach, describe, expect, it } from "vitest";
import { activate } from "../i18n/i18n";
import {
  forwardActions,
  forwardLine,
  forwardNote,
  forwardTag,
  plainTag,
  type ForwardView,
} from "./forward_view";

function view(overrides: Partial<ForwardView> = {}): ForwardView {
  return {
    mode: "signed",
    forwardable: true,
    origin: {
      author_did: "did:key:h7wyt195mrxos3bj1a4ihoq843ixkxnp6bt5mfrodgob3z6wm8w5e4ae",
      written_at_secs: 1_765_000_000,
      record_id: "rec_abc",
      is_you: false,
    },
    forward_count: 1,
    many_forwards: false,
    forwarded: true,
    ...overrides,
  };
}

describe("the forwarded line", () => {
  afterEach(async () => {
    await activate("en");
  });

  it("names a contact and counts the forwards", () => {
    const contacts = new Map([["did:key:h7wyt195mrxos3bj1a4ihoq843ixkxnp6bt5mfrodgob3z6wm8w5e4ae", "Alice"]]);
    const line = forwardLine(view(), contacts);
    expect(line?.textContent).toContain("Forwarded. First written by Alice on");
    expect(line?.textContent).toContain("Forwarded 1 time.");
    expect(line?.querySelector(".forward-author")?.textContent).toBe("Alice");
  });

  it("says you when the author is the reader", () => {
    const line = forwardLine(view({ origin: { ...view().origin!, is_you: true } }));
    expect(line?.querySelector(".forward-author")?.textContent).toBe("you");
  });

  it("shows the last 12 characters of an unknown DID with the whole DID in a tooltip", () => {
    const origin = view().origin!;
    const line = forwardLine(view());
    const name = line?.querySelector<HTMLElement>(".forward-author");
    expect(name?.textContent).toBe(`…${origin.author_did.slice(-12)} (not in your contacts)`);
    expect(name?.textContent).not.toContain(origin.author_did.slice(0, 16));
    expect(name?.title).toBe(origin.author_did);
  });

  it("shows the short DID for a contact with no display name", () => {
    const origin = view().origin!;
    const contacts = new Map([[origin.author_did, ""]]);
    const name = forwardLine(view(), contacts)?.querySelector<HTMLElement>(".forward-author");
    expect(name?.textContent).toBe(`…${origin.author_did.slice(-12)}`);
    expect(name?.textContent).not.toContain("(not in your contacts)");
    expect(name?.title).toBe(origin.author_did);
  });

  it("uses times for 4 and many times from 5", () => {
    expect(forwardLine(view({ forward_count: 4 }))?.textContent).toContain("Forwarded 4 times.");
    expect(
      forwardLine(view({ forward_count: 5, many_forwards: true }))?.textContent,
    ).toContain("Forwarded many times.");
  });

  it("is not drawn for a message that was not forwarded", () => {
    expect(forwardLine(view({ forwarded: false, forward_count: 0 }))).toBeNull();
    expect(forwardLine(undefined)).toBeNull();
  });
});

describe("a failed origin", () => {
  const failed = view({
    forwardable: false,
    origin: undefined,
    origin_error: "content hash does not match",
    forwarded: false,
    forward_count: 0,
  });

  it("never names an author and shows the note with its reason", () => {
    const line = forwardLine(failed);
    expect(line).toBeNull();
    const note = forwardNote(failed);
    expect(note?.textContent).toBe("origin could not be checked");
    expect(note?.title).toBe("content hash does not match");
    expect(note?.textContent).not.toContain("did:key");
  });

  it("offers no forward action", () => {
    expect(forwardActions({ id: "m1", forward: failed }, async () => {})).toBeNull();
  });
});

describe("the tags", () => {
  afterEach(async () => {
    await activate("en");
  });

  it("says Signed on a checked signed message", () => {
    expect(forwardTag({ content_type: "application/vnd.roym.message+json", forward: view() })?.textContent).toBe(
      "Signed",
    );
  });

  it("says why a not-forwardable message cannot be forwarded", () => {
    const tag = forwardTag({
      content_type: "application/vnd.roym.message+json",
      forward: view({ mode: "not-forwardable", forwardable: false, origin: undefined, forwarded: false }),
    });
    expect(tag?.textContent).toBe("Not forwardable (the author chose this)");
  });

  it("gives a plain message the no-origin tag and nothing else a tag", () => {
    expect(plainTag({ content_type: "text/plain" })?.textContent).toBe(
      "Cannot be forwarded (no signed origin)",
    );
    for (const content_type of [
      "application/vnd.roym.card+json",
      "application/vnd.roym.widget+json",
      "application/vnd.roym.widget-answer+json",
      "application/vnd.roym.membership-event+json",
    ]) {
      expect(plainTag({ content_type })).toBeNull();
      expect(forwardTag({ content_type })).toBeNull();
    }
  });

  it("draws no author tag for an unreadable body", () => {
    const unreadable = view({
      mode: "not-forwardable",
      forwardable: false,
      origin: undefined,
      origin_error: "message body could not be read",
      forwarded: false,
      forward_count: 0,
    });
    expect(forwardTag({ content_type: "application/vnd.roym.message+json", forward: unreadable })).toBeNull();
    expect(plainTag({ content_type: "application/vnd.roym.message+json" })).toBeNull();
  });
});

describe("the forward action", () => {
  it("is offered on a checked signed message", () => {
    const actions = forwardActions({ id: "m1", forward: view() }, async () => {});
    expect(actions?.querySelector(".forward-button")).not.toBeNull();
  });
});
