import { afterEach, describe, expect, it, vi } from "vitest";
import type { MergedHit } from "../directory/search";
import { membershipLine } from "./directory";

const NOW = Math.floor(Date.now() / 1000);

const HIT: MergedHit = {
  listing_id: "lst_1",
  record_id: "rec_1",
  issuer: "did:key:zProvider",
  title: "Jam",
  summary: "",
  categories: ["gardening"],
  conversation_address: "did:key:zConversation",
  status: "active",
  verified: true,
  revocation_status: "none",
  age_secs: 10,
  sources: [],
  versions_differ: false,
};

const OUT_OF_SCOPE = {
  state: "out-of-scope" as const,
  credential_record_id: "rec_c",
  outside: ["gardening"],
};

const VALID = {
  state: "valid" as const,
  credential_record_id: "rec_c",
  issuer: "did:key:zIssuer",
  synorg_name: "Canned Guild",
  scope: { categories: ["canning"], area: [] },
  expires_at_secs: NOW + 86400,
  revocations_checked_as_of_secs: NOW,
};

function stubCheckStanding(reply: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({ ok: true, json: async () => ({ result: reply }) })),
  );
}

function cardLine(): HTMLElement {
  return membershipLine(
    { directory: "did:key:zDirectory", record_id: "rec_1", received_at_secs: NOW, membership: OUT_OF_SCOPE },
    HIT,
  );
}

async function clickCheck(line: HTMLElement) {
  (line.querySelector(".check-membership") as HTMLButtonElement).click();
  await vi.waitFor(() =>
    expect(line.querySelector(".membership-check-status")?.textContent).not.toBe(""),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("a search card's membership line", () => {
  it("keeps the listing's own verdict when the person checks again", async () => {
    // `check-standing` judges the provider with no listing, so it says
    // "valid" for a credential that does not cover this listing's category.
    stubCheckStanding({ verdict: VALID, as_of_secs: NOW, refreshed: true });
    const line = cardLine();
    expect(line.querySelector(".evidence-membership")?.textContent).toContain(
      "Membership does not cover this listing: gardening",
    );

    await clickCheck(line);

    expect(line.querySelector(".evidence-membership")?.textContent).toContain(
      "Membership does not cover this listing: gardening",
    );
    const fresh = line.querySelector(".evidence-membership-fresh")?.textContent ?? "";
    expect(fresh).toContain("not tied to this listing");
    expect(fresh).toContain("Member of Canned Guild");
    expect(line.querySelector(".membership-check-status")?.textContent).toBe("Checked just now.");
  });

  it("says a changed issuer is a different group, not an unreachable directory", async () => {
    stubCheckStanding({ verdict: { state: "unknown", reason: "issuer-changed" }, refreshed: false });
    const line = cardLine();

    await clickCheck(line);

    const status = line.querySelector(".membership-check-status")?.textContent ?? "";
    expect(status).toContain("different group");
    expect(status).not.toContain("Could not reach");
    expect(line.querySelector(".evidence-membership-fresh")?.textContent).toBe("");
  });

  it("still says the directory could not be reached when it could not", async () => {
    stubCheckStanding({ verdict: VALID, as_of_secs: NOW - 7200, refreshed: false, error: "timed out" });
    const line = cardLine();

    await clickCheck(line);

    expect(line.querySelector(".membership-check-status")?.textContent).toContain(
      "Could not reach this directory. Showing what was checked 2 hours ago.",
    );
  });
});
