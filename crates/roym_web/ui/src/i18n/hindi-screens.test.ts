import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { renderAgreementReceipt } from "../cards/templates/agreement_receipt";
import { renderFulfilmentReceipt } from "../cards/templates/fulfilment_receipt";
import { renderQuote } from "../cards/templates/quote";
import { renderRequest } from "../cards/templates/request";
import { RpcError } from "../rpc";
import { renderBackup } from "../screens/backup";
import { renderContacts } from "../screens/contacts";
import { renderDirectory } from "../screens/directory";
import { renderGroups } from "../screens/groups";
import { renderListings } from "../screens/listings";
import { renderMemberships } from "../screens/memberships";
import { type CardRow, openDeclineDialog, openQuoteForm, renderMessages } from "../screens/messages";
import { renderProfile } from "../screens/profile";
import { renderSafety } from "../screens/safety";
import { renderSetup } from "../screens/setup";
import { renderSynOrg } from "../screens/synorg";
import { leftoverLatin } from "../testing/latin-words";
import { stubRpc } from "../testing/stub-rpc";
import { activate, t } from "./i18n";

// Draws each screen in Hindi with a fake `/rpc` and fails when an English
// word is left. A value that a person types or another node sends holds the
// marker `Zq` and is removed first. A code (a state, a status, a category of
// a report) is never marked, so a screen that shows one as it came is found.

const NOW = Math.floor(Date.now() / 1000);
const ME = "did:key:zZqMe";
const PEER = "did:key:zZqPeer";

beforeAll(async () => {
  await activate("hi");
});
afterAll(async () => {
  await activate("en");
});
afterEach(() => {
  vi.unstubAllGlobals();
});

/// Every text the person can read inside `host`: text nodes (which include
/// the options of a list), the placeholder, title and label attributes, and
/// the value that a field starts with.
function shown(host: HTMLElement): string[] {
  const out: string[] = [];
  const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const value = node.textContent?.trim();
    if (value) out.push(value);
  }
  for (const el of host.querySelectorAll<HTMLElement>("[placeholder],[title],[aria-label]")) {
    for (const name of ["placeholder", "title", "aria-label"]) {
      const value = el.getAttribute(name);
      if (value) out.push(value);
    }
  }
  for (const el of host.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input,textarea")) {
    if (el.type !== "checkbox" && el.value) out.push(el.value);
  }
  return out;
}

/// The English words left in `host`, as `text` lines for the failure message.
function english(host: HTMLElement): string[] {
  return shown(host).flatMap((text) => {
    const left = leftoverLatin(text);
    return left.length > 0 ? [`${JSON.stringify(text)} -> ${left.join(", ")}`] : [];
  });
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 20));
}

function envelope(payload: Record<string, unknown>, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({ record_id: "Zq-rec", issuer: ME, payload, ...extra });
}

const VALID_VERDICT = {
  state: "valid",
  credential_record_id: "Zq-cred",
  issuer: "did:key:zZqIssuer",
  synorg_name: "Zq-Guild",
  scope: { categories: ["bicycle-repair"], area: [] },
  expires_at_secs: NOW + 86400,
  revocations_checked_as_of_secs: NOW - 7200,
};

function message(id: string, state: string, extra: Record<string, unknown> = {}) {
  return {
    id,
    conversation: "c1",
    author: PEER,
    direction: "outgoing",
    sender_timestamp_ms: 1_800_000_000_000,
    content_type: "text/plain",
    body_encoding: "utf8",
    body: `Zq-body-${id}`,
    state,
    ...extra,
  };
}

describe("the small screens, in Hindi", () => {
  it("draws the profile screen and its saved line", async () => {
    stubRpc({
      "profile.get": () => ({
        envelope: envelope({
          display_name: "Zq-Alice",
          conversation_address: "Zq-addr",
          lang: "hi",
          languages: ["hi", "ta"],
        }),
      }),
      "profile.set": () => ({ record_id: "Zq-rec", envelope: envelope({}) }),
    });
    const host = document.createElement("div");
    await renderProfile(host);
    host.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await settle();
    expect(english(host)).toEqual([]);
  });

  it("draws the contacts screen and a bad envelope", async () => {
    stubRpc({
      "contacts.list": () => [{ person_did: PEER, display_name: "Zq-Bob", favourite: true }],
    });
    const host = document.createElement("div");
    await renderContacts(host);
    host.querySelector("textarea")!.value = "Zq-not-json";
    host.querySelector<HTMLButtonElement>("button.contacts-upsert-button")!.click();
    await settle();
    expect(english(host)).toEqual([]);
  });

  it("draws the contacts screen with a search result and empty message", async () => {
    stubRpc({
      "contacts.list": (params: unknown) => {
        if (
          params &&
          typeof params === "object" &&
          "text" in params &&
          (params as { text: string }).text === "Zq-query"
        ) {
          return [{ person_did: PEER, display_name: "Zq-Bob", favourite: true }];
        }
        if (
          params &&
          typeof params === "object" &&
          "text" in params &&
          (params as { text: string }).text === "Zq-none"
        ) {
          return [];
        }
        return [{ person_did: PEER, display_name: "Zq-Bob", favourite: true }];
      },
    });
    const host = document.createElement("div");
    await renderContacts(host);
    const searchInput = host.querySelector<HTMLInputElement>(".contacts-search-input")!;
    const searchBtn = host.querySelector<HTMLButtonElement>(".contacts-search-button")!;

    searchInput.value = "Zq-query";
    searchBtn.click();
    await settle();
    expect(english(host)).toEqual([]);

    searchInput.value = "Zq-none";
    searchBtn.click();
    await settle();
    expect(host.textContent).toContain(t("contacts-search-empty"));
    expect(english(host)).toEqual([]);
  });

  it("draws the backup and set-up screens", async () => {
    const backup = document.createElement("div");
    await renderBackup(backup);
    const setup = document.createElement("div");
    await renderSetup(setup, () => {}, ["Zq-service"]);
    expect(english(backup)).toEqual([]);
    expect(english(setup)).toEqual([]);
  });

  it("draws the safety screen with an open and a withdrawn report", async () => {
    stubRpc({
      "profile.policy": () => ({}),
      "block.list": () => [{ person_did: PEER }],
      "report.list": () => [
        { report_id: "r1", subject_kind: "person", subject_id: "Zq-id", category: "fraud", status: "open" },
        { report_id: "r2", subject_kind: "listing", subject_id: "Zq-id", category: "harassment", status: "withdrawn" },
      ],
      "contacts.limits": () => ({ window_secs: 60, max_per_window: 3 }),
    });
    const host = document.createElement("div");
    await renderSafety(host);
    await settle();
    expect(english(host)).toEqual([]);
  });
});

describe("the messages screen, in Hindi", () => {
  const CONVERSATIONS = [
    { id: "c1", peer_address: PEER, peer_person_did: PEER, last_activity_ms: 1, message_count: 5 },
  ];

  function stubMessages(): void {
    stubRpc({
      "conversation.list": () => ({ conversations: CONVERSATIONS }),
      "transaction.sync": () => ({}),
      "transaction.thread": () => ({ cards: [] }),
      "conversation.history": () => ({
        messages: [
          message("m1", "delivered", { languages: ["hi"] }),
          message("m2", "deposited"),
          message("m3", "failed", { last_error: "Zq-error", refused: "rate-limited" }),
          message("m4", "pending", { direction: "incoming", refused: "Zq-reason" }),
          message("m5", "delivered", { deleted_at_secs: 5 }),
        ],
      }),
    });
  }

  it("draws the list, the thread, the states and the request form", async () => {
    stubMessages();
    const host = document.createElement("div");
    await renderMessages(host, ME);
    host.querySelector<HTMLButtonElement>("button.toggle-request-form")?.click();
    const filter = host.querySelector<HTMLInputElement>(".request-cats-input .picker-filter")!;
    filter.value = "bicycle";
    filter.dispatchEvent(new Event("input"));
    const opt = host.querySelector<HTMLButtonElement>(
      '.request-cats-input .picker-option[data-code="bicycle-repair"]',
    )!;
    opt.click();
    expect(
      host.querySelectorAll('.request-cats-input .picker-chip[data-code="bicycle-repair"]'),
    ).toHaveLength(1);
    expect(host.querySelectorAll(".message")).toHaveLength(5);
    expect(english(host)).toEqual([]);
  });

  it("draws both delete notes", async () => {
    stubMessages();
    const host = document.createElement("div");
    await renderMessages(host, ME);
    const buttons = host.querySelectorAll<HTMLButtonElement>("button.delete-message");
    buttons[0].click();
    buttons[3].click();
    expect(host.querySelectorAll(".delete-note")).toHaveLength(2);
    expect(english(host)).toEqual([]);
  });

  const REQUEST_CARD = { record_id: "Zq-req", card_type: "request" } as CardRow;

  it("draws the quote form and its first errors", async () => {
    const host = document.createElement("div");
    openQuoteForm(host, REQUEST_CARD, "c1", async () => {});
    host.querySelector<HTMLButtonElement>("button.submit-quote-button")!.click();
    await settle();
    expect(host.querySelector(".quote-form-error")?.textContent).not.toBe("");
    expect(english(host)).toEqual([]);
  });

  it("draws the decline dialog and a failure", async () => {
    stubRpc({ "quote.decline": () => new RpcError(-32603, "Fake service error", "Other") });
    const host = document.createElement("div");
    openDeclineDialog(host, REQUEST_CARD, async () => {});
    host.querySelector<HTMLButtonElement>("button.confirm-decline")!.click();
    await settle();
    expect(host.querySelector(".decline-error")?.textContent).toContain("Fake service error");
    expect(english(host)).toEqual([]);
  });
});

describe("the groups screen, in Hindi", () => {
  it("draws a group with members, messages and a hidden group", async () => {
    stubRpc({
      "conversation.list": () => ({
        conversations: [
          { id: "g1", kind: "group", message_count: 3, group: { name: "Zq-Group", admission: { state: "shown" } } },
          { id: "g2", kind: "group", message_count: 0, group: { name: null, admission: { state: "hidden" } } },
        ],
      }),
      "group.info": () => ({
        conversation_id: "g1",
        name: "Zq-Group",
        owner_address: "did:key:zZqOwner",
        owner_person_did: ME,
        is_owner: true,
        is_member: true,
        restored_only: false,
        epoch: 1,
        key_epoch: 2,
        key_stored_at_ms: 1_800_000_000_000,
        members: [
          { address: "did:key:zZqOwner", person_did: ME, is_owner: true },
          { address: "did:key:zZqMember", person_did: PEER, is_owner: false },
        ],
        can_read_new_messages: true,
      }),
      "conversation.transcript-digest": () => ({ digest: "Zq1234567890abcdef" }),
      "contacts.list": () => [],
      "conversation.history": () => ({
        messages: [
          message("g-m1", "delivered", { conversation: "g1", direction: "outgoing", languages: ["hi"] }),
          message("g-m2", "failed", { conversation: "g1" }),
          message("g-m3", "delivered", { conversation: "g1", deleted_at_secs: 5 }),
          message("g-m4", "delivered", {
            conversation: "g1",
            content_type: "application/vnd.roym.group-profile+json",
            body: JSON.stringify({ name: "Zq-Named" }),
          }),
        ],
      }),
    });
    const host = document.createElement("div");
    await renderGroups(host);
    await settle();
    expect(host.querySelector(".group-title")).not.toBeNull();
    expect(english(host)).toEqual([]);
  });
});

describe("the listing screen, in Hindi", () => {
  it("draws an active and a draft listing, its slots and the editor", async () => {
    const row = (status: string) => ({
      listing_id: `Zq-${status}`,
      slug: "Zq-slug",
      title: "Zq-title",
      status,
      record_id: "Zq-rec",
      updated_at_secs: NOW,
      version_count: 2,
    });
    stubRpc({
      "listing.list": () => ({ listings: [row("active"), row("draft")] }),
      "availability.list": () => ({
        slots: [{ slot_id: "s1", start_secs: NOW, end_secs: NOW + 3600, capacity: 2 }],
      }),
    });
    const host = document.createElement("div");
    await renderListings(host);
    const filter = host.querySelector<HTMLInputElement>(".listing-categories-input .picker-filter")!;
    filter.focus();
    filter.value = "bicycle";
    filter.dispatchEvent(new Event("input"));
    const opt = host.querySelector<HTMLButtonElement>(
      '.listing-categories-input .picker-option[data-code="bicycle-repair"]',
    )!;
    opt.click();
    expect(
      host.querySelectorAll('.listing-categories-input .picker-chip[data-code="bicycle-repair"]'),
    ).toHaveLength(1);
    host.querySelector<HTMLButtonElement>("button.listing-translation-add-btn")!.click();
    expect(host.querySelectorAll(".listing-translation-row")).toHaveLength(1);
    host.querySelector<HTMLButtonElement>("button.save-listing")!.click();
    host.querySelector<HTMLButtonElement>("button.verify-listing")!.click();
    await settle();
    expect(host.querySelectorAll(".listing-row")).toHaveLength(2);
    expect(host.querySelector(".listing-save-result")?.textContent).toMatch(/[ऀ-ॿ]/);
    expect(english(host)).toEqual([]);
  });
});

describe("the directory screens, in Hindi", () => {
  const SOURCES = [
    { did: "did:key:zZqDirA", label: "Zq-A", last_error: { kind: "timed-out" } },
    { did: "did:key:zZqDirB", label: "Zq-B", last_ok_secs: NOW },
  ];

  it("draws the directories, a search with hits, refusals and source errors", async () => {
    stubRpc({
      "directory.sources": () => ({ sources: SOURCES }),
      "directory.run-envelope": () => ({ envelope: "Zq-envelope" }),
      "directory.find": () => ({
        run_id: "Zq-run",
        outcomes: [
          { source: "did:key:zZqDirA", verified: 0, refused: 0, error: { kind: "timed-out" } },
          { source: "did:key:zZqDirB", verified: 1, refused: 1, truncated: true },
        ],
        hits: [
          {
            listing_id: "Zq-l",
            record_id: "Zq-r",
            issuer: "did:key:zZqOwner",
            title: "Zq-title",
            summary: "Zq-summary",
            categories: ["bicycle-repair"],
            conversation_address: "Zq-addr",
            status: "active",
            verified: true,
            revocation_status: "good",
            age_secs: 3600,
            lang: "hi",
            translations: [{ lang: "en", title: "Zq-en-title", summary: "Zq-en-summary" }],
            sources: [
              { directory: "did:key:zZqDirB", record_id: "Zq-r", received_at_secs: NOW, membership: VALID_VERDICT },
            ],
            versions_differ: true,
          },
        ],
        hits_truncated: true,
        refused: [{ listing_id: "Zq-x", reason: "", sources: ["did:key:zZqDirA"] }],
        refused_truncated: true,
      }),
    });
    const host = document.createElement("div");
    await renderDirectory(host);
    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(host.querySelector(".search-hit")).not.toBeNull());
    host.querySelector<HTMLButtonElement>("button.hit-engage")!.click();
    await settle();
    expect(english(host)).toEqual([]);
  });

  it("draws the memberships screen with a valid and a revoked verdict", async () => {
    const revoked = {
      state: "revoked",
      credential_record_id: "Zq-cred",
      revocation_record_id: "Zq-rev",
      revoked_at_secs: NOW - 86400,
      reason: "Zq-reason",
    };
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.memberships": () => ({
        memberships: [
          { source: "did:key:zZqDirA", member_did: ME, issuer_did: "did:key:zZqIssuer", as_of_secs: NOW - 100, verdict: VALID_VERDICT },
          { source: "did:key:zZqDirB", member_did: PEER, issuer_did: null, as_of_secs: 0, last_error: "Zq-error", verdict: revoked },
        ],
      }),
    });
    const host = document.createElement("div");
    await renderMemberships(host);
    expect(english(host)).toEqual([]);
  });
});

describe("the SynOrg screen, in Hindi", () => {
  const SETTINGS = {
    name: "Zq-Guild",
    rules: "Zq-rules",
    area: [],
    categories: ["bicycle-repair", "x9"],
    support_contact: "Zq-support",
    dispute_path: "Zq-dispute",
    retention_secs: 30 * 86400,
    publication_limits: { window_secs: 86400, max_per_window: 20 },
  };

  function credential(id: string, issuedAt: number) {
    return {
      record_id: id,
      member_did: PEER,
      about: "",
      issued_at_secs: issuedAt,
      envelope: JSON.stringify({
        expires_at_secs: NOW + 86400 * 30,
        payload: { scope: { categories: ["bicycle-repair"], area: [] } },
      }),
    };
  }

  function decision(id: string, action: string, about: string) {
    return {
      record_id: id,
      member_did: PEER,
      about,
      issued_at_secs: NOW - 100,
      envelope: JSON.stringify({
        payload: {
          action,
          scope: { kind: "listing", listing_id: "Zq-listing" },
          rule: "Zq-rule",
          reason: "Zq-reason",
          until_secs: NOW + 86400,
        },
      }),
    };
  }

  it("draws the settings, the members panel and the publications", async () => {
    stubRpc({
      "directory.settings": () => SETTINGS,
      "member.list": () => ({ members: [{ did: PEER, note: "Zq-note" }] }),
      "directory.publications": () => ({
        publications: [{ listing_id: "Zq-listing", issuer: PEER, published_by: ME }],
      }),
      "credential.list": () => ({
        records: [credential("Zq-c0", NOW - 300), credential("Zq-c1", NOW - 200), credential("Zq-c2", NOW - 100)],
      }),
      "revocation.list": () => ({
        records: [{ record_id: "Zq-rv", member_did: PEER, about: "Zq-c1", issued_at_secs: NOW, envelope: "{}" }],
      }),
      "member.decisions": () => ({
        records: [decision("Zq-d1", "suspend", ""), decision("Zq-d2", "suspend", ""), decision("Zq-d3", "lift", "Zq-d2")],
      }),
    });
    const host = document.createElement("div");
    await renderSynOrg(host);
    const statuses = [...host.querySelectorAll(".credential-row")].map((el) => (el as HTMLElement).dataset.status);
    expect(statuses.sort()).toEqual(["current", "replaced", "revoked"]);
    expect(english(host)).toEqual([]);
  });

  it("draws the empty form for an installation with no SynOrg", async () => {
    stubRpc({ "directory.settings": () => null });
    const host = document.createElement("div");
    await renderSynOrg(host);
    expect(english(host)).toEqual([]);
  });
});

describe("the cards, in Hindi", () => {
  it("draws the request card", () => {
    const el = renderRequest({
      lang: "hi",
      description: "Zq-desc",
      categories: ["bicycle-repair"],
      data_use_notice: "Zq-notice",
      window: { earliest_secs: NOW, latest_secs: NOW + 3600 },
    });
    expect(english(el)).toEqual([]);
  });

  it("draws the quote card with a translation", () => {
    const el = renderQuote({
      terms: {
        lang: "hi",
        scope: "Zq-scope",
        amount_minor: 1000,
        currency: "INR",
        location: { where: "at-receiver", address: "Zq-addr" },
        cancellation_terms: "Zq-canc",
        refund_terms: "Zq-refund",
        dispute_path: "Zq-disp",
        quote_expires_at_secs: NOW + 86400,
      },
      translations: [
        {
          lang: "en",
          scope: "Zq-en-scope",
          cancellation_terms: "Zq-en-canc",
          refund_terms: "Zq-en-refund",
          dispute_path: "Zq-en-disp",
        },
      ],
    });
    expect(english(el)).toEqual([]);
  });

  it("draws the quote card showing an author translation in Hindi with binds warning", () => {
    const el = renderQuote({
      terms: {
        lang: "en",
        scope: "Zq-scope",
        amount_minor: 1000,
        currency: "INR",
        location: { where: "at-receiver", address: "Zq-addr" },
        cancellation_terms: "Zq-canc",
        refund_terms: "Zq-refund",
        dispute_path: "Zq-disp",
        quote_expires_at_secs: NOW + 86400,
      },
      translations: [
        {
          lang: "hi",
          scope: "Zq-hi-scope",
          cancellation_terms: "Zq-hi-canc",
          refund_terms: "Zq-hi-refund",
          dispute_path: "Zq-hi-disp",
        },
      ],
    });
    expect(el.querySelector(".quote-original-binds")?.textContent).toMatch(/[ऀ-ॿ]/);
    expect(el.querySelector(".record-text-note")?.textContent).toMatch(/[ऀ-ॿ]/);
    expect(english(el)).toEqual([]);
  });

  it("draws the agreement receipt card", () => {
    const el = renderAgreementReceipt({
      role: "counterparty",
      pair: { state: "complete" },
      counterparty_did: "did:key:zZqCounterparty",
      quote_author_did: "did:key:zZqQuoteAuthor",
      quote_record_id: "Zq-rec",
      terms: {
        lang: "hi",
        scope: "Zq-scope",
        amount_minor: 1000,
        currency: "INR",
        location: { where: "at-receiver", address: "Zq-addr" },
        cancellation_terms: "Zq-canc",
        refund_terms: "Zq-refund",
        dispute_path: "Zq-disp",
        quote_expires_at_secs: NOW + 86400,
      },
    });
    expect(english(el)).toEqual([]);
  });

  it("draws the fulfilment receipt card", () => {
    const el = renderFulfilmentReceipt({
      role: "quote-author",
      terms: {
        lang: "hi",
        scope: "Zq-scope",
      },
    });
    expect(english(el)).toEqual([]);
  });
});

describe("the sweep itself", () => {
  it("finds the English words of a screen drawn in English", async () => {
    await activate("en");
    try {
      const host = document.createElement("div");
      await renderBackup(host);
      expect(english(host).length).toBeGreaterThan(3);
    } finally {
      await activate("hi");
    }
  });

  it("finds a code that a screen shows as it came", () => {
    const host = document.createElement("div");
    const raw = document.createElement("span");
    raw.textContent = "delivered";
    host.append(raw);
    expect(english(host)).toEqual(['"delivered" -> delivered']);
  });

  it("finds a raw taxonomy code left on a screen", () => {
    const host = document.createElement("div");
    const raw = document.createElement("span");
    raw.textContent = "bicycle-repair";
    host.append(raw);
    expect(english(host)).toEqual(['"bicycle-repair" -> bicycle, repair']);
  });
});
