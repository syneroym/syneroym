import { afterEach, describe, expect, it, vi } from "vitest";
import { activate } from "../i18n/i18n";
import {
  parseRecordForward,
  recordForwardAction,
  recordForwardBlock,
  type RecordForwardBody,
} from "./record_forward";

const ENVELOPE = '{"envelope_version":1,"record_type":"quote"}';

function quoteVerdict() {
  return {
    verified: true,
    issuer: "did:key:zQuoteAuthor",
    payload: {
      quote_id: "q1",
      conversation: "conv-1",
      sequence: 1,
      counterparty_did: "did:key:zCounterparty",
      terms: {
        currency: "INR",
        quote_expires_at_secs: 1_796_000_000,
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            resource_kind: "goods-or-service",
            kind: "fulfilment",
            _sd: [],
          },
          {
            commitment_id: "c2",
            giver: "counterparty",
            resource_kind: "currency",
            kind: "currency",
            _sd: [],
          },
        ],
        _sd: [],
      },
    },
  };
}

interface Call {
  method: string;
  params: Record<string, unknown>;
}

function stubRpc(
  reply: (method: string, params: Record<string, unknown>) => unknown,
  calls: Call[] = [],
) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as {
        method: string;
        params: Record<string, unknown>;
      };
      calls.push({ method: body.method, params: body.params });
      return { ok: true, json: async () => reply(body.method, body.params) };
    }),
  );
}

describe("the record-forward body", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("reads a body of this version and refuses anything else", () => {
    const good = JSON.stringify({
      record_forward_version: 1,
      record_type: "listing",
      envelope: ENVELOPE,
    });
    expect(parseRecordForward(good)?.record_type).toBe("listing");

    for (const bad of [
      undefined,
      "",
      "not json",
      JSON.stringify({ record_forward_version: 2, record_type: "listing", envelope: ENVELOPE }),
      JSON.stringify({ record_forward_version: 1, record_type: "card", envelope: ENVELOPE }),
      JSON.stringify({ record_forward_version: 1, record_type: "listing", envelope: "" }),
      JSON.stringify({ record_forward_version: 1, record_type: "listing" }),
    ]) {
      expect(parseRecordForward(bad)).toBeNull();
    }
  });

  it("draws a quote's visible fields and never its value or a you-give line", async () => {
    stubRpc((method) => {
      if (method === "quote.verify") return { result: quoteVerdict() };
      throw new Error(`unexpected ${method}`);
    });
    const body: RecordForwardBody = {
      record_forward_version: 1,
      record_type: "quote",
      envelope: ENVELOPE,
    };
    const box = recordForwardBlock(body);
    await vi.waitFor(() =>
      expect(box.querySelector(".record-forward-quote-currency")).not.toBeNull(),
    );

    const text = box.textContent ?? "";
    expect(text).toContain("did:key:zQuoteAuthor");
    expect(text).toContain("did:key:zCounterparty");
    expect(text).toContain("Currency: INR");
    expect(text).toContain("Expires:");
    const commitments = box.querySelectorAll(".record-forward-quote-commitment");
    expect(commitments[0].textContent).toBe(
      "Commitment c1: Goods or service from did:key:zQuoteAuthor",
    );
    expect(commitments[1].textContent).toBe(
      "Commitment c2: Money from did:key:zCounterparty",
    );
    expect(box.querySelector(".record-forward-quote-hidden")).not.toBeNull();
    expect(text).not.toContain("You give");
    expect(text).not.toContain("₹");
    expect(box.querySelector(".quote-total")).toBeNull();
  });

  it("draws a received listing from its verdict", async () => {
    stubRpc((method) => {
      if (method === "listing.verify") {
        return {
          result: {
            verified: true,
            payload: {
              title: "Garden fence",
              summary: "Repaired in a day",
              categories: ["gardening", "repair"],
              status: "open",
            },
          },
        };
      }
      throw new Error(`unexpected ${method}`);
    });
    const box = recordForwardBlock({
      record_forward_version: 1,
      record_type: "listing",
      envelope: ENVELOPE,
    });
    await vi.waitFor(() =>
      expect(box.querySelector(".record-forward-title")?.textContent).toBe("Garden fence"),
    );
    expect(box.querySelector(".record-forward-summary")?.textContent).toBe("Repaired in a day");
    expect(box.textContent).toContain("gardening, repair");
    expect(box.querySelector(".record-forward-status-line")?.textContent).toBe("open");
  });

  it("draws a received profile from its verdict", async () => {
    stubRpc((method) => {
      if (method === "profile.verify-envelope") {
        return {
          result: {
            verified: true,
            payload: { display_name: "Alice", about: "Hindi tutor" },
          },
        };
      }
      throw new Error(`unexpected ${method}`);
    });
    const box = recordForwardBlock({
      record_forward_version: 1,
      record_type: "profile",
      envelope: ENVELOPE,
    });
    await vi.waitFor(() =>
      expect(box.querySelector(".record-forward-name")?.textContent).toBe("Alice"),
    );
    expect(box.querySelector(".record-forward-about")?.textContent).toBe("Hindi tutor");
  });

  it("draws no field when the record does not check, and keeps the reason", async () => {
    stubRpc(() => ({ result: { verified: false, reason: "bad signature" } }));
    const box = recordForwardBlock({
      record_forward_version: 1,
      record_type: "profile",
      envelope: ENVELOPE,
    });
    const note = await vi.waitFor(() => {
      const el = box.querySelector<HTMLElement>(".record-forward-not-checked");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(note.title).toBe("bad signature");
    expect(box.querySelector(".record-forward-name")).toBeNull();
    expect(box.textContent).not.toContain("did:key:");
  });
});

describe("the record-forward action", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("warns before a quote and sends the envelope only after Continue", async () => {
    const calls: Call[] = [];
    stubRpc(
      (method) => {
        if (method === "quote.verify") return { result: quoteVerdict() };
        if (method === "conversation.list") {
          return { result: { conversations: [{ id: "conv-1", peer_address: "peer" }] } };
        }
        if (method === "conversation.forward-record") return { result: { message_id: "m1" } };
        throw new Error(`unexpected ${method}`);
      },
      calls,
    );

    const host = document.createElement("div");
    host.appendChild(
      recordForwardAction({
        type: "quote",
        loadEnvelope: async () => ENVELOPE,
      }),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-button")!.click();

    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-warning")).not.toBeNull(),
    );
    expect(host.querySelector(".forward-picker")).toBeNull();
    expect(host.querySelector(".record-forward-warning")?.textContent).toContain(
      "shows both people of the deal",
    );

    host.querySelector<HTMLButtonElement>(".record-forward-continue")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".forward-picker-row")).not.toBeNull(),
    );
    host.querySelector<HTMLButtonElement>(".forward-picker-row")!.click();
    await vi.waitFor(() =>
      expect(calls.some((c) => c.method === "conversation.forward-record")).toBe(true),
    );
    const sent = calls.find((c) => c.method === "conversation.forward-record");
    expect(sent?.params).toEqual({
      conversation: "conv-1",
      record_type: "quote",
      envelope: ENVELOPE,
    });
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-status")?.textContent).toBe("Forwarded."),
    );
  });

  it("cancels the quote warning, sends nothing, and shows one warning again", async () => {
    const calls: Call[] = [];
    stubRpc(
      (method) => {
        if (method === "quote.verify") return { result: quoteVerdict() };
        if (method === "conversation.list") return { result: { conversations: [] } };
        throw new Error(`unexpected ${method}`);
      },
      calls,
    );

    const host = document.createElement("div");
    host.appendChild(
      recordForwardAction({
        type: "quote",
        loadEnvelope: async () => ENVELOPE,
      }),
    );
    const forward = host.querySelector<HTMLButtonElement>(".record-forward-button")!;
    forward.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-warning")).not.toBeNull(),
    );

    host.querySelector<HTMLButtonElement>(".record-forward-cancel")!.click();
    expect(host.querySelector(".record-forward-warning")).toBeNull();
    expect(calls.some((c) => c.method === "conversation.forward-record")).toBe(false);

    forward.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-warning")).not.toBeNull(),
    );
    forward.click();
    await vi.waitFor(() =>
      expect(host.querySelectorAll(".record-forward-warning")).toHaveLength(1),
    );
  });

  it("keeps the confirmation outside a thread that redraws", async () => {
    stubRpc((method) => {
      if (method === "quote.verify") return { result: quoteVerdict() };
      if (method === "conversation.list") {
        return { result: { conversations: [{ id: "conv-1", peer_address: "peer" }] } };
      }
      if (method === "conversation.forward-record") return { result: { message_id: "m1" } };
      throw new Error(`unexpected ${method}`);
    });

    const statusHost = document.createElement("div");
    const host = document.createElement("div");
    document.body.append(statusHost, host);
    host.appendChild(
      recordForwardAction({
        type: "quote",
        loadEnvelope: async () => ENVELOPE,
        statusHost,
      }),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-button")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-warning")).not.toBeNull(),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-continue")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".forward-picker-row")).not.toBeNull(),
    );
    host.querySelector<HTMLButtonElement>(".forward-picker-row")!.click();
    await vi.waitFor(() =>
      expect(statusHost.querySelector(".record-forward-status")?.textContent).toBe("Forwarded."),
    );

    host.replaceChildren();
    expect(statusHost.querySelector(".record-forward-status")?.textContent).toBe("Forwarded.");
    host.remove();
    statusHost.remove();
  });

  it("offers the picker at once for a listing", async () => {
    stubRpc((method) => {
      if (method === "listing.verify") return { result: { verified: true, payload: {} } };
      if (method === "conversation.list") return { result: { conversations: [] } };
      throw new Error(`unexpected ${method}`);
    });
    const host = document.createElement("div");
    host.appendChild(
      recordForwardAction({
        type: "listing",
        loadEnvelope: async () => ENVELOPE,
      }),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-button")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".forward-picker-empty")).not.toBeNull(),
    );
    expect(host.querySelector(".record-forward-warning")).toBeNull();
  });

  it("does not offer a record that does not check", async () => {
    stubRpc((method) => {
      if (method === "profile.verify-envelope") {
        return { result: { verified: false, reason: "no certificate" } };
      }
      throw new Error(`unexpected ${method}`);
    });
    const host = document.createElement("div");
    host.appendChild(
      recordForwardAction({
        type: "profile",
        loadEnvelope: async () => ENVELOPE,
      }),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-button")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-not-checked")).not.toBeNull(),
    );
    expect(host.querySelector(".forward-picker")).toBeNull();
    expect(
      host.querySelector<HTMLElement>(".record-forward-not-checked")?.title,
    ).toBe("no certificate");
  });

  it("says when the record is no longer on this node", async () => {
    const host = document.createElement("div");
    host.appendChild(
      recordForwardAction({
        type: "profile",
        loadEnvelope: async () => null,
      }),
    );
    host.querySelector<HTMLButtonElement>(".record-forward-button")!.click();
    await vi.waitFor(() =>
      expect(host.querySelector(".record-forward-status")?.textContent).toBe(
        "This record is no longer on this node.",
      ),
    );
  });
});
