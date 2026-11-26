import { describe, expect, it } from "vitest";
import { pollDefinition } from "./view";

describe("pollDefinition", () => {
  it("builds the definition matching the Rust fixture shape", () => {
    const def = pollDefinition("Which day shall we meet?", ["Saturday", "Sunday"]);
    expect(def).toEqual({
      extension: { id: "poll", version: 1 },
      type: "poll/single",
      semantic: "poll",
      prompt: "Which day shall we meet?",
      fields: [
        {
          name: "option",
          type: "choice",
          required: true,
          choices: [
            { code: "o1", text: "Saturday" },
            { code: "o2", text: "Sunday" },
          ],
        },
      ],
    });
  });
});
