import { afterEach, describe, expect, it } from "vitest";

import { waitingRedraw } from "./thread_redraw";

const OPEN = ".quote-form, .decline-dialog";

function setUp() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  let calls = 0;
  const redraw = waitingRedraw(host, async () => {
    calls++;
  }, OPEN);
  return { host, redraw, calls: () => calls };
}

function openElement(host: HTMLElement, className: string): HTMLElement {
  const el = document.createElement("div");
  el.className = className;
  host.appendChild(el);
  return el;
}

/** Let the `MutationObserver` callbacks run: they are microtasks. */
async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("waitingRedraw", () => {
  it("redraws at once when nothing is open", async () => {
    const { redraw, calls } = setUp();
    await redraw.later();
    expect(calls()).toBe(1);
  });

  it("waits while open work is there, and redraws once when it leaves", async () => {
    const { host, redraw, calls } = setUp();
    const form = openElement(host, "quote-form");
    await redraw.later();
    await redraw.later();
    await redraw.later();
    await settle();
    expect(calls()).toBe(0);
    form.remove();
    await settle();
    expect(calls()).toBe(1);
  });

  it("waits until the last open work leaves", async () => {
    const { host, redraw, calls } = setUp();
    const form = openElement(host, "quote-form");
    const dialog = openElement(host, "decline-dialog");
    await redraw.later();
    form.remove();
    await settle();
    expect(calls()).toBe(0);
    dialog.remove();
    await settle();
    expect(calls()).toBe(1);
  });

  it("now redraws at once and drops the waiting redraw", async () => {
    const { host, redraw, calls } = setUp();
    const form = openElement(host, "quote-form");
    await redraw.later();
    await redraw.now();
    expect(calls()).toBe(1);
    form.remove();
    await settle();
    expect(calls()).toBe(1);
  });

  it("a change that is not open work does not redraw", async () => {
    const { host, calls } = setUp();
    const plain = openElement(host, "message");
    plain.remove();
    await settle();
    expect(calls()).toBe(0);
  });
});
