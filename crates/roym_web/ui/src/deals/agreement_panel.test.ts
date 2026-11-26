import { afterEach, describe, expect, it, vi } from "vitest";
import { stubRpc } from "../testing/stub-rpc";
import {
  type AgreementView,
  renderAgreementPanel,
  renderAgreementPanelView,
} from "./agreement_panel";
import { type CardRow, messageElement } from "../screens/messages";

describe("agreement panel", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  const baseView: AgreementView = {
    quote_record_id: "rec_agr_1",
    quote_id: "quo_1",
    conversation: "conv_1",
    quote_author_did: "did:key:alice_author",
    counterparty_did: "did:key:bob_counterparty",
    state: "agreed",
    terms: {
      currency: "INR",
      commitments: [
        {
          commitment_id: "c1",
          giver: "quote-author",
          kind: "fulfilment",
          resource_kind: "goods-or-service",
          description: "Guitar lesson",
          declared_value_minor: 1500,
        },
        {
          commitment_id: "c2",
          giver: "counterparty",
          kind: "currency",
          resource_kind: "currency",
          description: "INR 15.00",
          declared_value_minor: 1500,
        },
      ],
    },
    commitments: [
      {
        commitment_id: "c1",
        state: "promised",
      },
      {
        commitment_id: "c2",
        state: "promised",
      },
    ],
  };

  it("shows 'I delivered' for giver of fulfilment and 'Received' for receiver", async () => {
    const calls = stubRpc({
      "fulfilment.sign": () => ({ state: "sent" }),
    });

    let reloaded = false;
    let refreshed = false;
    const el = renderAgreementPanelView(
      baseView,
      "did:key:alice_author", // Alice is author (giver of c1)
      async () => {
        refreshed = true;
      },
      async () => {
        reloaded = true;
      },
    );

    document.body.appendChild(el);

    const c1Row = el.querySelector(".agreement-commitment-c1");
    expect(c1Row?.querySelector(".agreement-commitment-giver")?.textContent).toBe("I give");
    const deliveredBtn = c1Row?.querySelector<HTMLButtonElement>(".deal-action-delivered-button");
    expect(deliveredBtn).not.toBeNull();
    expect(deliveredBtn?.textContent).toBe("I delivered");

    // c2 is currency from counterparty: Alice is receiver of c2
    const c2Row = el.querySelector(".agreement-commitment-c2");
    expect(c2Row?.querySelector(".agreement-commitment-giver")?.textContent).toBe("They give");
    const receivedBtn = c2Row?.querySelector<HTMLButtonElement>(".deal-action-received-button");
    expect(receivedBtn).not.toBeNull();
    expect(receivedBtn?.textContent).toBe("Received");

    // Click "I delivered" opens settle-dialog
    deliveredBtn?.click();
    const dialog = c1Row?.querySelector(".settle-dialog");
    expect(dialog).not.toBeNull();

    const noteInput = dialog?.querySelector<HTMLInputElement>(".settle-note-input");
    if (noteInput) noteInput.value = "Handed in person";

    const confirmBtn = dialog?.querySelector<HTMLButtonElement>(".settle-confirm-button");
    confirmBtn?.click();

    await new Promise((r) => setTimeout(r, 10));

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("fulfilment.sign");
    expect(calls[0].params.agreement).toBe("rec_agr_1");
    expect(calls[0].params.commitment_id).toBe("c1");
    expect(calls[0].params.settlement_note).toEqual({ lang: "en", text: "Handed in person" });
    expect(reloaded).toBe(true);
    expect(refreshed).toBe(true);
  });

  it("shows 'I paid' for giver of currency", async () => {
    const calls = stubRpc({
      "payment.acknowledge": () => ({ state: "sent" }),
    });

    const el = renderAgreementPanelView(
      baseView,
      "did:key:bob_counterparty", // Bob is counterparty (giver of c2)
      async () => {},
      async () => {},
    );

    const c2Row = el.querySelector(".agreement-commitment-c2");
    const paidBtn = c2Row?.querySelector<HTMLButtonElement>(".deal-action-paid-button");
    expect(paidBtn).not.toBeNull();
    expect(paidBtn?.textContent).toBe("I paid");

    paidBtn?.click();
    const dialog = c2Row?.querySelector(".settle-dialog");
    expect(dialog).not.toBeNull();

    dialog?.querySelector<HTMLButtonElement>(".settle-confirm-button")?.click();
    await new Promise((r) => setTimeout(r, 10));

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("payment.acknowledge");
    expect(calls[0].params.commitment_id).toBe("c2");
  });

  it("shows 'You said you delivered' when giver claimed, but receiver still sees 'Received'", () => {
    const claimedView: AgreementView = {
      ...baseView,
      commitments: [
        {
          commitment_id: "c1",
          state: "claimed",
          claimed: true,
        },
        {
          commitment_id: "c2",
          state: "promised",
        },
      ],
    };

    // Alice as giver
    const elAlice = renderAgreementPanelView(claimedView, "did:key:alice_author", async () => {}, async () => {});
    const c1Alice = elAlice.querySelector(".agreement-commitment-c1");
    expect(c1Alice?.querySelector(".deal-claimed-giver")?.textContent).toBe("You said you delivered");
    expect(c1Alice?.querySelector(".deal-action-delivered-button")).toBeNull();

    // Bob as receiver
    const elBob = renderAgreementPanelView(claimedView, "did:key:bob_counterparty", async () => {}, async () => {});
    const c1Bob = elBob.querySelector(".agreement-commitment-c1");
    expect(c1Bob?.querySelector(".deal-action-received-button")).not.toBeNull();
  });

  it("shows 'Sent. Waiting for the other person's node.' on counterparty node when half is signed locally", () => {
    const waitingView: AgreementView = {
      ...baseView,
      commitments: [
        {
          commitment_id: "c1",
          state: "promised",
          halves: ["rec_half_signed_by_bob"],
        },
        {
          commitment_id: "c2",
          state: "promised",
        },
      ],
    };

    const el = renderAgreementPanelView(
      waitingView,
      "did:key:bob_counterparty",
      async () => {},
      async () => {},
    );

    const c1Row = el.querySelector(".agreement-commitment-c1");
    expect(c1Row?.querySelector(".deal-waiting-other-node")?.textContent).toBe(
      "Sent. Waiting for the other person's node.",
    );
    expect(c1Row?.querySelector(".deal-action-received-button")).toBeNull();
  });

  it("renders ended-unconfirmed with AG-8 two sentences", () => {
    const unconfirmedView: AgreementView = {
      ...baseView,
      commitments: [
        {
          commitment_id: "c1",
          state: "ended-unconfirmed",
          claimed: true,
        },
        {
          commitment_id: "c2",
          state: "ended-unconfirmed",
          claimed: false,
        },
      ],
    };

    const el = renderAgreementPanelView(unconfirmedView, "did:key:alice_author", async () => {}, async () => {});
    const c1State = el.querySelector(".agreement-commitment-c1 .agreement-commitment-state");
    expect(c1State?.textContent).toBe("The giver says it delivered; the receiver did not confirm.");

    const c2State = el.querySelector(".agreement-commitment-c2 .agreement-commitment-state");
    expect(c2State?.textContent).toBe("Nobody recorded anything.");
  });

  it("closing actions: 'I gave everything' and 'I received everything' with closeList", async () => {
    const calls = stubRpc({
      "agreement.close": () => ({ records: [], skipped: [] }),
    });

    const el = renderAgreementPanelView(baseView, "did:key:alice_author", async () => {}, async () => {});
    document.body.appendChild(el);

    const giveAllBtn = el.querySelector<HTMLButtonElement>(".deal-close-give-button");
    expect(giveAllBtn?.textContent).toBe("I gave everything");

    const receiveAllBtn = el.querySelector<HTMLButtonElement>(".deal-close-receive-button");
    expect(receiveAllBtn?.textContent).toBe("I received everything");

    // Click "I gave everything" opens .close-list
    giveAllBtn?.click();
    const closeList = el.querySelector(".close-list");
    expect(closeList).not.toBeNull();

    const cb = closeList?.querySelector<HTMLInputElement>(".close-list-item-checkbox");
    expect(cb?.checked).toBe(true);

    const signBtn = closeList?.querySelector<HTMLButtonElement>(".deal-close-sign-button");
    expect(signBtn?.textContent).toBe("Sign");

    signBtn?.click();
    await new Promise((r) => setTimeout(r, 10));

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("agreement.close");
    expect(calls[0].params.as).toBe("giver");
    expect(calls[0].params.commitments).toEqual(["c1"]);
  });

  it("renderAgreementPanel loads from agreement.get and redraws", async () => {
    stubRpc({
      "agreement.get": () => baseView,
    });

    const host = renderAgreementPanel("rec_agr_1", "did:key:alice_author", async () => {});
    document.body.appendChild(host);

    await new Promise((r) => setTimeout(r, 10));

    const panel = document.querySelector(".agreement-panel-rec_agr_1");
    expect(panel).not.toBeNull();
    expect(panel?.querySelector(".agreement-commitment-c1")).not.toBeNull();
  });

  it("thread wiring: agreement panel is rendered below newest card of agreement", async () => {
    stubRpc({
      "agreement.get": () => baseView,
    });

    const cardRow: CardRow = {
      message_id: "m_receipt_1",
      conversation: "conv_1",
      direction: "incoming",
      sender_timestamp_ms: 1000,
      card_type: "agreement-receipt",
      version: 1,
      known: true,
      verified: true,
      expired: false,
      record_id: "rec_receipt_1",
      data: {
        quote_record_id: "rec_agr_1",
        agreement: "rec_agr_1",
      },
      stored_at_secs: 1,
    };

    const cardMap = new Map<string, CardRow>([["m_receipt_1", cardRow]]);
    const allCards = [cardRow];
    const newestReqMap = new Map<string, string>();
    const newestQuoteMap = new Map<string, string>();
    const newestAgrMap = new Map<string, string>([["rec_agr_1", "m_receipt_1"]]);

    const msg = {
      id: "m_receipt_1",
      conversation: "conv_1",
      author: "did:key:alice_author",
      direction: "incoming" as const,
      content_type: "application/vnd.roym.card+json",
      state: "delivered" as const,
      sender_timestamp_ms: 1000,
      body_encoding: "utf8" as const,
    };

    const el = messageElement(
      msg,
      cardMap,
      allCards,
      newestReqMap,
      newestQuoteMap,
      "conv_1",
      "did:key:alice_author",
      async () => {},
      undefined,
      undefined,
      undefined,
      newestAgrMap,
    );

    document.body.appendChild(el);

    const panel = el.querySelector(".agreement-panel-rec_agr_1");
    expect(panel).not.toBeNull();
  });
});
