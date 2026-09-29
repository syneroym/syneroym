import { afterEach, describe, expect, it, vi } from "vitest";
import { NO_INSTANT_REMOVAL_NOTICE, WITHHELD_REVOCATION_NOTICE } from "../directory/membership";
import { renderMemberships } from "./memberships";

type Handler = (params: Record<string, unknown>) => unknown;

/// A fake `/rpc` that answers each method with the handler registered for
/// it, and records every call.
function stubRpc(handlers: Record<string, Handler>) {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as {
        method: string;
        params: Record<string, unknown>;
      };
      calls.push(body);
      const handler = handlers[body.method];
      if (!handler) throw new Error(`unexpected method ${body.method}`);
      return { ok: true, json: async () => ({ result: handler(body.params) }) };
    }),
  );
  return calls;
}

const NOW = Math.floor(Date.now() / 1000);

const VALID = {
  state: "valid",
  credential_record_id: "rec_c",
  issuer: "did:key:zIssuer",
  synorg_name: "Cycling Guild",
  scope: { categories: ["cycling"], area: [] },
  expires_at_secs: NOW + 86400,
  revocations_checked_as_of_secs: NOW - 7200,
};

const HELD = {
  source: "did:key:zDirectory",
  member_did: "did:key:zProvider",
  issuer_did: "did:key:zIssuer",
  as_of_secs: NOW - 7200,
  verdict: VALID,
};

afterEach(() => {
  vi.unstubAllGlobals();
});

async function render(): Promise<HTMLElement> {
  const host = document.createElement("div");
  await renderMemberships(host);
  return host;
}

describe("the Memberships screen", () => {
  it("keeps both notices on screen even with nothing held", async () => {
    stubRpc({ "directory.memberships": () => ({ memberships: [] }), "directory.sources": () => ({ sources: [] }) });
    const host = await render();
    expect(host.querySelector(".no-instant-removal-notice")?.textContent).toBe(NO_INSTANT_REMOVAL_NOTICE);
    expect(host.querySelector(".withheld-revocation-notice")?.textContent).toBe(WITHHELD_REVOCATION_NOTICE);
    expect(host.querySelector(".memberships-empty")).not.toBeNull();
  });

  it("shows the held copy with its age and the issuer as the directory first said it", async () => {
    stubRpc({
      "directory.memberships": () => ({ memberships: [HELD] }),
      "directory.sources": () => ({ sources: [{ did: HELD.source, label: "Guild directory" }] }),
    });
    const host = await render();
    const row = host.querySelector(".membership-row") as HTMLElement;
    expect(row.querySelector(".membership-source")?.textContent).toContain("Guild directory");
    expect(row.querySelector(".membership-words")?.textContent).toContain("Member of Cycling Guild");
    expect(row.querySelector(".membership-checked")?.textContent).toBe("checked 2 hours ago");
    expect(row.querySelector(".membership-issuer")?.textContent).toBe(
      "The group this directory said it is when you added it: did:key:zIssuer",
    );
    expect(row.textContent?.toLowerCase()).not.toContain("verified");
  });

  it("re-reads the group on Check again and shows the new verdict as of now", async () => {
    const calls = stubRpc({
      "directory.memberships": () => ({ memberships: [HELD] }),
      "directory.sources": () => ({ sources: [] }),
      "directory.check-standing": () => ({
        verdict: {
          state: "suspended",
          decision_record_id: "rec_d",
          scope: { kind: "membership" },
          rule: "r1",
          reason: "late",
          since_secs: NOW,
        },
        as_of_secs: NOW,
        refreshed: true,
      }),
    });
    const host = await render();
    const row = host.querySelector(".membership-row") as HTMLElement;
    (row.querySelector(".check-again") as HTMLButtonElement).click();
    await vi.waitFor(() => expect(row.dataset.state).toBe("suspended"));

    expect(calls.find((c) => c.method === "directory.check-standing")?.params).toEqual({
      source: HELD.source,
      member_did: HELD.member_did,
    });
    expect(row.querySelector(".membership-words")?.textContent).toContain("Membership suspended");
    expect(row.querySelector(".membership-checked")?.textContent).toBe("checked moments ago");
    expect(row.querySelector(".membership-check-status")?.textContent).toBe("Checked just now.");
  });

  it("says the old copy was kept when the directory could not be reached", async () => {
    stubRpc({
      "directory.memberships": () => ({ memberships: [HELD] }),
      "directory.sources": () => ({ sources: [] }),
      "directory.check-standing": () => ({ verdict: VALID, as_of_secs: HELD.as_of_secs, refreshed: false }),
    });
    const host = await render();
    const row = host.querySelector(".membership-row") as HTMLElement;
    (row.querySelector(".check-again") as HTMLButtonElement).click();
    await vi.waitFor(() =>
      expect(row.querySelector(".membership-check-status")?.textContent).toContain("copy you already held"),
    );
    expect(row.querySelector(".membership-checked")?.textContent).toBe("checked 2 hours ago");
  });

  it("renders a hostile group name as text, never as markup", async () => {
    const hostile = { ...VALID, synorg_name: "<img src=x onerror=window.evil=1>" };
    stubRpc({
      "directory.memberships": () => ({ memberships: [{ ...HELD, verdict: hostile }] }),
      "directory.sources": () => ({ sources: [] }),
    });
    const host = await render();
    expect(host.querySelector("img")).toBeNull();
    expect(host.querySelector(".membership-words")?.textContent).toContain("<img src=x");
  });
});
