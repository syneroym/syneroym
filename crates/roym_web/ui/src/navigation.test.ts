import { describe, expect, it, vi } from "vitest";
import { openTab, setTabOpener } from "./navigation";

describe("navigation", () => {
  it("openTab calls registered opener with tab and options", async () => {
    const opener = vi.fn().mockResolvedValue(undefined);
    setTabOpener(opener);

    await openTab("messages", { conversationId: "c1" });
    expect(opener).toHaveBeenCalledWith("messages", { conversationId: "c1" });

    setTabOpener(null);
    await openTab("directory");
    expect(opener).toHaveBeenCalledTimes(1);
  });
});
