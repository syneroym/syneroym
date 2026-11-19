import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activate, t } from "../i18n/i18n";
import { RpcError } from "../rpc";
import { stubRpc } from "../testing/stub-rpc";
import { attachSizeLine, type MeasureResult } from "./quote_size";

describe("quote_size", () => {
  beforeEach(async () => {
    await activate("en");
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("shows used and limit when quote fits and mailbox fits", async () => {
    stubRpc({
      "quote.measure": () => ({
        bytes: 2150,
        limit: 96 * 1024,
        details: [],
        fits: true,
        card_bytes: 5000,
        mailbox_fits: true,
      } satisfies MeasureResult),
    });

    const form = document.createElement("form");
    const terms = { scope: "Lesson" };
    attachSizeLine(form, () => terms);

    await vi.advanceTimersByTimeAsync(300);

    const line = form.querySelector<HTMLElement>(".quote-size-line")!;
    expect(line).not.toBeNull();
    expect(line.style.display).toBe("");
    expect(line.className).toBe("quote-size-line");
    expect(line.textContent).toBe("Size: 2.1 KiB of 96 KiB");
  });

  it("adds over class and text when fits is false", async () => {
    stubRpc({
      "quote.measure": () => ({
        bytes: 100 * 1024,
        limit: 96 * 1024,
        details: [],
        fits: false,
        card_bytes: 120 * 1024,
        mailbox_fits: false,
      } satisfies MeasureResult),
    });

    const form = document.createElement("form");
    const terms = { scope: "Huge quote" };
    attachSizeLine(form, () => terms);

    await vi.advanceTimersByTimeAsync(300);

    const line = form.querySelector<HTMLElement>(".quote-size-line")!;
    expect(line.style.display).toBe("");
    expect(line.className).toBe("quote-size-line quote-size-over");
    expect(line.textContent).toBe("Size: 100.0 KiB of 96 KiB " + t("quote-size-over"));
  });

  it("shows mailbox note when fits is true and mailbox_fits is false", async () => {
    stubRpc({
      "quote.measure": () => ({
        bytes: 50 * 1024,
        limit: 96 * 1024,
        details: [],
        fits: true,
        card_bytes: 80 * 1024,
        mailbox_fits: false,
      } satisfies MeasureResult),
    });

    const form = document.createElement("form");
    const terms = { scope: "Medium quote" };
    attachSizeLine(form, () => terms);

    await vi.advanceTimersByTimeAsync(300);

    const line = form.querySelector<HTMLElement>(".quote-size-line")!;
    expect(line.style.display).toBe("");
    expect(line.className).toBe("quote-size-line");
    expect(line.textContent).toBe("Size: 50.0 KiB of 96 KiB\n" + t("quote-size-mailbox"));
  });

  it("hides line on a parse error (readTerms returns undefined)", async () => {
    let callCount = 0;
    stubRpc({
      "quote.measure": () => {
        callCount++;
        return {
          bytes: 2000,
          limit: 96 * 1024,
          details: [],
          fits: true,
          card_bytes: 4000,
          mailbox_fits: true,
        } satisfies MeasureResult;
      },
    });

    const form = document.createElement("form");
    attachSizeLine(form, () => undefined);

    await vi.advanceTimersByTimeAsync(300);

    const line = form.querySelector<HTMLElement>(".quote-size-line")!;
    expect(line.style.display).toBe("none");
    expect(line.textContent).toBe("");
    expect(callCount).toBe(0);
  });

  it("hides line on signing-not-enrolled error", async () => {
    stubRpc({
      "quote.measure": () => new RpcError(-32602, "signing-not-enrolled", "Other"),
    });

    const form = document.createElement("form");
    attachSizeLine(form, () => ({ scope: "Lesson" }));

    await vi.advanceTimersByTimeAsync(300);

    const line = form.querySelector<HTMLElement>(".quote-size-line")!;
    expect(line.style.display).toBe("none");
    expect(line.textContent).toBe("");
  });

  it("makes only one call after typing stops", async () => {
    let callCount = 0;
    stubRpc({
      "quote.measure": () => {
        callCount++;
        return {
          bytes: 2000,
          limit: 96 * 1024,
          details: [],
          fits: true,
          card_bytes: 4000,
          mailbox_fits: true,
        } satisfies MeasureResult;
      },
    });

    const form = document.createElement("form");
    const input = document.createElement("input");
    form.appendChild(input);

    attachSizeLine(form, () => ({ scope: "Lesson" }));

    for (let i = 0; i < 5; i++) {
      form.dispatchEvent(new Event("input"));
      await vi.advanceTimersByTimeAsync(50);
    }
    expect(callCount).toBe(0);

    await vi.advanceTimersByTimeAsync(300);
    expect(callCount).toBe(1);
  });
});
