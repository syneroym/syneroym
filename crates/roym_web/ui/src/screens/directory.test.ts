import { afterEach, describe, expect, it, vi } from "vitest";
import HINDI from "../i18n/hi.ftl?raw";
import { activate, activateWith } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import type { MergedHit } from "../directory/search";
import { membershipLine, renderDirectory } from "./directory";

const NOW = Math.floor(Date.now() / 1000);

const HIT: MergedHit = {
  listing_id: "lst_1",
  record_id: "rec_1",
  issuer: "did:key:zListingOwner",
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
  lang: "en",
  translations: [],
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
    // `check-standing` judges the listing owner with no listing, so it says
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

describe("the revocation line of a search hit", () => {
  afterEach(async () => {
    await activate("en");
  });

  async function revocationLine(status: string): Promise<string> {
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.find": () => ({
        run_id: "run",
        outcomes: [],
        hits: [{ ...HIT, revocation_status: status }],
        hits_truncated: false,
        refused: [],
        refused_truncated: false,
      }),
    });
    const host = document.createElement("div");
    await renderDirectory(host);
    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(host.querySelector(".evidence-revocation")).not.toBeNull());
    return host.querySelector(".evidence-revocation")!.textContent ?? "";
  }

  it("shows the status as it is in English", async () => {
    expect(await revocationLine("good")).toBe("revocation: good");
    expect(await revocationLine("unknown")).toBe("revocation: unknown");
  });

  it("shows a Hindi word for a known status, and any other status as it came", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    expect(await revocationLine("good")).not.toContain("good");
    expect(await revocationLine("good")).toMatch(/[ऀ-ॿ]/);
    for (const status of ["constructor", "toString", "__proto__"]) {
      expect(await revocationLine(status)).toContain(status);
    }
  });
});

describe("category search and hit display", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("a category search sends the same query in English and in Hindi", async () => {
    let capturedQueryEn: Record<string, unknown> | undefined;
    let capturedQueryHi: Record<string, unknown> | undefined;

    // English
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.find": (params) => {
        capturedQueryEn = params as Record<string, unknown>;
        return {
          run_id: "run-en",
          outcomes: [],
          hits: [],
          hits_truncated: false,
          refused: [],
          refused_truncated: false,
        };
      },
    });
    const hostEn = document.createElement("div");
    await renderDirectory(hostEn);
    const filterEn = hostEn.querySelector<HTMLInputElement>(".search-categories .picker-filter")!;
    filterEn.value = "Bicycle repair";
    filterEn.dispatchEvent(new Event("input"));
    const optEn = hostEn.querySelector<HTMLButtonElement>('.search-categories .picker-option[data-code="bicycle-repair"]')!;
    optEn.click();
    hostEn.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(capturedQueryEn).toBeDefined());
    const queryEn = capturedQueryEn?.query as Record<string, unknown>;
    expect(queryEn?.categories).toEqual(["bicycle-repair"]);

    // Hindi
    await activate("hi");
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.find": (params) => {
        capturedQueryHi = params as Record<string, unknown>;
        return {
          run_id: "run-hi",
          outcomes: [],
          hits: [],
          hits_truncated: false,
          refused: [],
          refused_truncated: false,
        };
      },
    });
    const hostHi = document.createElement("div");
    await renderDirectory(hostHi);
    const filterHi = hostHi.querySelector<HTMLInputElement>(".search-categories .picker-filter")!;
    filterHi.value = "साइकिल";
    filterHi.dispatchEvent(new Event("input"));
    const optHi = hostHi.querySelector<HTMLButtonElement>('.search-categories .picker-option[data-code="bicycle-repair"]')!;
    optHi.click();
    hostHi.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(capturedQueryHi).toBeDefined());
    const queryHi = capturedQueryHi?.query as Record<string, unknown>;
    expect(queryHi?.categories).toEqual(["bicycle-repair"]);

    expect(queryHi?.categories).toEqual(queryEn?.categories);
  });

  it("shows hit categories as labels and an own code as it came", async () => {
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.find": () => ({
        run_id: "run-labels",
        outcomes: [],
        hits: [{ ...HIT, categories: ["bicycle-repair", "my-own-category"] }],
        hits_truncated: false,
        refused: [],
        refused_truncated: false,
      }),
    });
    const host = document.createElement("div");
    await renderDirectory(host);
    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(host.querySelector(".hit-categories")).not.toBeNull());
    expect(host.querySelector(".hit-categories")!.textContent).toBe(
      "Categories: Bicycle Repair, my-own-category",
    );
  });

  it("a directory hit with a translation shows the best version and the note; a hit without one shows no note", async () => {
    stubRpc({
      "directory.sources": () => ({ sources: [] }),
      "directory.find": () => ({
        run_id: "run-trans",
        outcomes: [],
        hits: [
          {
            ...HIT,
            listing_id: "l_trans",
            lang: "hi",
            title: "झाड़ी कटाई",
            summary: "बगीचे की सफाई",
            translations: [
              {
                lang: "en",
                title: "Hedge trimming",
                summary: "Garden cleanup",
              },
            ],
          },
          {
            ...HIT,
            listing_id: "l_notrans",
            lang: "en",
            title: "Plumbing",
            summary: "Pipe fixing",
            translations: [],
          },
        ],
        hits_truncated: false,
        refused: [],
        refused_truncated: false,
      }),
    });
    const host = document.createElement("div");
    await renderDirectory(host);
    host.querySelector<HTMLButtonElement>("button.run-search")!.click();
    await vi.waitFor(() => expect(host.querySelectorAll(".search-hit")).toHaveLength(2));

    const hits = host.querySelectorAll(".search-hit");
    // Hit 1 with translation, reader in English: shows English title and summary + note
    expect(hits[0].querySelector(".hit-title")?.textContent).toBe("Hedge trimming");
    expect(hits[0].querySelector(".hit-title")?.getAttribute("lang")).toBe("en");
    expect(hits[0].querySelector(".hit-summary")?.textContent).toBe("Garden cleanup");
    expect(hits[0].querySelector(".hit-summary")?.getAttribute("lang")).toBe("en");
    expect(hits[0].querySelector(".record-text-note")?.textContent).toBe("Translated by the author from Hindi.");
    expect(hits[0].querySelector(".record-text-switch")?.textContent).toBe("Show the original");

    // Hit 2 without translation: shows original, no note or switch
    expect(hits[1].querySelector(".hit-title")?.textContent).toBe("Plumbing");
    expect(hits[1].querySelector(".hit-title")?.getAttribute("lang")).toBe("en");
    expect(hits[1].querySelector(".record-text-note")).toBeNull();
    expect(hits[1].querySelector(".record-text-switch")).toBeNull();
  });
});
