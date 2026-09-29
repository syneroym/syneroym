import { afterEach, describe, expect, it, vi } from "vitest";
import { NO_INSTANT_REMOVAL_NOTICE } from "../directory/membership";
import { buildMembersPanel } from "./synorg_members";

type Handler = (params: Record<string, unknown>) => unknown;

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
      const out = handler(body.params);
      if (out instanceof Error) {
        return { ok: true, json: async () => ({ error: { code: -32602, message: out.message } }) };
      }
      return { ok: true, json: async () => ({ result: out }) };
    }),
  );
  return calls;
}

const MEMBER = "did:key:zMember";
const NOW = Math.floor(Date.now() / 1000);

function credentialRow(id: string, issuedAt: number) {
  return {
    record_id: id,
    member_did: MEMBER,
    about: "",
    issued_at_secs: issuedAt,
    envelope: JSON.stringify({
      expires_at_secs: NOW + 86400 * 30,
      payload: { scope: { categories: ["cycling"], area: [] } },
    }),
  };
}

function decisionRow(id: string, action: "suspend" | "lift", about: string, issuedAt: number) {
  return {
    record_id: id,
    member_did: MEMBER,
    about,
    issued_at_secs: issuedAt,
    envelope: JSON.stringify({
      payload: { action, scope: { kind: "membership" }, rule: action === "lift" ? "" : "r1", reason: "t" },
    }),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the SynOrg members panel", () => {
  it("shows the no-instant-removal notice above every control", async () => {
    stubRpc({
      "credential.list": () => ({ records: [] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    expect(panel.querySelector(".no-instant-removal-notice")?.textContent).toBe(NO_INSTANT_REMOVAL_NOTICE);
  });

  it("marks a credential current, replaced or revoked, and offers Revoke only on the current one", async () => {
    stubRpc({
      "credential.list": () => ({
        records: [credentialRow("rec_old", NOW - 500), credentialRow("rec_new", NOW - 100)],
      }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    const rows = [...panel.querySelectorAll<HTMLElement>(".credential-row")];
    expect(rows.map((r) => r.dataset.status)).toEqual(["current", "replaced"]);
    expect(panel.querySelectorAll(".revoke-credential")).toHaveLength(1);
  });

  it("marks a revoked credential revoked and offers no second Revoke", async () => {
    stubRpc({
      "credential.list": () => ({ records: [credentialRow("rec_c", NOW - 100)] }),
      "revocation.list": () => ({
        records: [{ record_id: "rec_r", member_did: MEMBER, about: "rec_c", issued_at_secs: NOW, envelope: "{}" }],
      }),
      "member.decisions": () => ({ records: [] }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    expect((panel.querySelector(".credential-row") as HTMLElement).dataset.status).toBe("revoked");
    expect(panel.querySelector(".revoke-credential")).toBeNull();
  });

  it("revokes the credential it names, with the typed reason", async () => {
    const calls = stubRpc({
      "credential.list": () => ({ records: [credentialRow("rec_c", NOW - 100)] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
      "revocation.issue": () => ({ record_id: "rec_r" }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    (panel.querySelector(".revoke-reason") as HTMLInputElement).value = "left the guild";
    (panel.querySelector(".revoke-credential") as HTMLButtonElement).click();
    await vi.waitFor(() => expect(calls.some((c) => c.method === "revocation.issue")).toBe(true));
    expect(calls.find((c) => c.method === "revocation.issue")?.params).toEqual({
      credential_record_id: "rec_c",
      reason: "left the guild",
    });
  });

  it("issues a credential for the chosen categories and days, and refuses bad input first", async () => {
    const calls = stubRpc({
      "credential.list": () => ({ records: [] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
      "credential.issue": () => ({ record_id: "rec_c" }),
    });
    const panel = await buildMembersPanel(["cycling", "plumbing"]);
    const click = () => (panel.querySelector(".issue-credential") as HTMLButtonElement).click();
    const status = () => panel.querySelector(".issue-status")?.textContent;

    click();
    expect(status()).toBe("Enter the member's DID.");
    (panel.querySelector(".issue-member-did") as HTMLInputElement).value = MEMBER;
    (panel.querySelector(".issue-days") as HTMLInputElement).value = "9999";
    click();
    expect(status()).toBe("Days must be a whole number from 1 to 730.");
    expect(calls.some((c) => c.method === "credential.issue")).toBe(false);

    (panel.querySelector(".issue-days") as HTMLInputElement).value = "30";
    (panel.querySelectorAll<HTMLInputElement>(".issue-category")[1]).checked = false;
    click();
    await vi.waitFor(() => expect(status()).toBe("Issued."));
    const issued = calls.find((c) => c.method === "credential.issue")!.params;
    expect(issued.member_did).toBe(MEMBER);
    expect(issued.categories).toEqual(["cycling"]);
    const days = ((issued.expires_at_secs as number) - NOW) / 86400;
    expect(Math.round(days)).toBe(30);
  });

  it("shows a refusal from the directory beside the button, in its own words", async () => {
    stubRpc({
      "credential.list": () => ({ records: [] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
      "credential.issue": () => new Error("categories are not all this SynOrg's own"),
    });
    const panel = await buildMembersPanel(["cycling"]);
    (panel.querySelector(".issue-member-did") as HTMLInputElement).value = MEMBER;
    (panel.querySelector(".issue-credential") as HTMLButtonElement).click();
    await vi.waitFor(() =>
      expect(panel.querySelector(".issue-status")?.textContent).toBe(
        "Not issued: categories are not all this SynOrg's own",
      ),
    );
  });

  it("offers Lift only on a suspension nothing has lifted, and lifts that one", async () => {
    const calls = stubRpc({
      "credential.list": () => ({ records: [] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({
        records: [
          decisionRow("rec_s1", "suspend", "", NOW - 300),
          decisionRow("rec_l1", "lift", "rec_s1", NOW - 200),
          decisionRow("rec_s2", "suspend", "", NOW - 100),
        ],
      }),
      "member.lift": () => ({ record_id: "rec_l2" }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    const rows = [...panel.querySelectorAll<HTMLElement>(".decision-row")];
    expect(rows.map((r) => r.dataset.status)).toEqual(["active", "lift", "lifted"]);
    expect(panel.querySelectorAll(".lift-suspension")).toHaveLength(1);
    (panel.querySelector(".lift-suspension") as HTMLButtonElement).click();
    await vi.waitFor(() => expect(calls.some((c) => c.method === "member.lift")).toBe(true));
    expect(calls.find((c) => c.method === "member.lift")?.params).toEqual({ decision_record_id: "rec_s2" });
  });

  it("suspends the whole membership, or one listing, with a rule and an optional end", async () => {
    const calls = stubRpc({
      "credential.list": () => ({ records: [] }),
      "revocation.list": () => ({ records: [] }),
      "member.decisions": () => ({ records: [] }),
      "member.suspend": () => ({ record_id: "rec_s" }),
    });
    const panel = await buildMembersPanel(["cycling"]);
    const set = (cls: string, v: string) => ((panel.querySelector(cls) as HTMLInputElement).value = v);
    const click = () => (panel.querySelector(".suspend-member") as HTMLButtonElement).click();

    click();
    expect(panel.querySelector(".suspend-status")?.textContent).toBe("A member DID and the rule are required.");

    set(".suspend-member-did", MEMBER);
    set(".suspend-rule", "r1");
    set(".suspend-reason", "late");
    click();
    await vi.waitFor(() => expect(panel.querySelector(".suspend-status")?.textContent).toBe("Suspended."));
    expect(calls.find((c) => c.method === "member.suspend")?.params).toEqual({
      member_did: MEMBER,
      rule: "r1",
      reason: "late",
    });

    set(".suspend-listing-id", "lst_x");
    set(".suspend-days", "7");
    click();
    await vi.waitFor(() => expect(calls.filter((c) => c.method === "member.suspend")).toHaveLength(2));
    const second = calls.filter((c) => c.method === "member.suspend")[1].params;
    expect(second.scope).toEqual({ kind: "listing", listing_id: "lst_x" });
    expect(Math.round(((second.until_secs as number) - NOW) / 86400)).toBe(7);
  });
});
