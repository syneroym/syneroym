import { afterEach, describe, expect, it, vi } from "vitest";
import { type CardRow, renderCardActions } from "./messages";
import { stubRpc } from "../testing/stub-rpc";

describe("suggest changes and send new version", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  const quoteV2: CardRow = {
    message_id: "m_q2",
    conversation: "conv1",
    direction: "incoming",
    sender_timestamp_ms: 2000,
    card_type: "quote",
    version: 1,
    known: true,
    verified: true,
    expired: false,
    record_id: "rec_q2",
    version_count: 2,
    data: {
      quote_id: "quo_1",
      sequence: 2,
      counterparty_did: "did:key:buyer",
      quote_author_did: "did:key:seller",
      terms: {
        currency: "INR",
        scope: "Guitar lessons",
        commitments: [
          {
            giver: "quote-author",
            kind: "fulfilment",
            resource_kind: "goods-or-service",
            description: "Guitar lessons",
            declared_value_minor: 1200,
          },
          {
            giver: "counterparty",
            kind: "currency",
            resource_kind: "currency",
            description: "INR 12.00",
            declared_value_minor: 1200,
          },
        ],
      },
    },
    stored_at_secs: 2,
  };

  it("suggest changes puts prefix in compose box and does not send", () => {
    const calls = stubRpc({});

    // Create a mock chat UI with a compose input
    const chatPane = document.createElement("div");
    chatPane.className = "chat-pane";
    const composeInput = document.createElement("input");
    composeInput.className = "compose-input";
    chatPane.appendChild(composeInput);

    const wrap = document.createElement("div");
    wrap.className = "message message-incoming";
    chatPane.appendChild(wrap);
    document.body.appendChild(chatPane);

    renderCardActions(wrap, quoteV2, "conv1", "did:key:buyer", async () => {});

    const suggestBtn = wrap.querySelector<HTMLButtonElement>(".quote-suggest-changes-button");
    expect(suggestBtn).not.toBeNull();
    expect(suggestBtn?.textContent).toBe("Suggest changes");

    // Click suggest changes
    suggestBtn?.click();

    expect(composeInput.value).toBe("About offer version 2: ");
    expect(document.activeElement).toBe(composeInput);
    // Nothing sent
    expect(calls.length).toBe(0);
  });

  it("send new version is available for author and opens prefilled quote form", () => {
    stubRpc({
      "deal.defaults-get": null,
      "quote.measure": { bytes: 500, limit: 1024, fits: true, mailbox_fits: true, details: [] },
    });

    const wrap = document.createElement("div");
    document.body.appendChild(wrap);

    renderCardActions(wrap, quoteV2, "conv1", "did:key:seller", async () => {});

    const newVerBtn = wrap.querySelector<HTMLButtonElement>(".quote-new-version-button");
    expect(newVerBtn).not.toBeNull();
    expect(newVerBtn?.textContent).toBe("Send a new version");

    // Click send new version
    newVerBtn?.click();

    const form = wrap.querySelector<HTMLElement>(".quote-form");
    expect(form).not.toBeNull();

    // Scope should be prefilled
    const scopeInput = form?.querySelector<HTMLInputElement>(".quote-scope-input");
    expect(scopeInput?.value).toBe("Guitar lessons");
  });
});
