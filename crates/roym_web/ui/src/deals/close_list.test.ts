import { describe, expect, it } from "vitest";
import { type AgreementViewForClose, closeList } from "./close_list";

describe("closeList", () => {
  const now = 1000;
  const nextWeek = now + 7 * 86400;

  it("unchecks a commitment whose window starts next week, and checks started ones", () => {
    const view: AgreementViewForClose = {
      role: "quote-author",
      terms: {
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            declared_value_minor: 500,
            window: { earliest_secs: now - 3600, latest_secs: now + 3600 },
          },
          {
            commitment_id: "c2",
            giver: "quote-author",
            declared_value_minor: 800,
            window: { earliest_secs: nextWeek, latest_secs: nextWeek + 3600 },
          },
        ],
      },
      commitments: [
        { commitment_id: "c1", state: "promised" },
        { commitment_id: "c2", state: "promised" },
      ],
    };

    const list = closeList(view, "give", now);
    expect(list).toHaveLength(2);

    expect(list[0].commitment_id).toBe("c1");
    expect(list[0].checked).toBe(true);
    expect(list[0].notStarted).toBe(false);

    expect(list[1].commitment_id).toBe("c2");
    expect(list[1].checked).toBe(false);
    expect(list[1].notStarted).toBe(true);
  });

  it("does not list final commitments", () => {
    const view: AgreementViewForClose = {
      role: "quote-author",
      terms: {
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            declared_value_minor: 500,
          },
          {
            commitment_id: "c2",
            giver: "quote-author",
            declared_value_minor: 600,
          },
          {
            commitment_id: "c3",
            giver: "quote-author",
            declared_value_minor: 700,
          },
        ],
      },
      commitments: [
        { commitment_id: "c1", state: "fulfilled" }, // final
        { commitment_id: "c2", state: "acknowledged" }, // final
        { commitment_id: "c3", state: "claimed" }, // open
      ],
    };

    const list = closeList(view, "give", now);
    expect(list).toHaveLength(1);
    expect(list[0].commitment_id).toBe("c3");
    expect(list[0].checked).toBe(true);
  });

  it("filters correctly by receiving side", () => {
    const view: AgreementViewForClose = {
      role: "quote-author",
      terms: {
        commitments: [
          {
            commitment_id: "c1",
            giver: "quote-author",
            declared_value_minor: 500,
          },
          {
            commitment_id: "c2",
            giver: "counterparty",
            declared_value_minor: 600,
          },
        ],
      },
    };

    const list = closeList(view, "receive", now);
    expect(list).toHaveLength(1);
    expect(list[0].commitment_id).toBe("c2");
  });
});
