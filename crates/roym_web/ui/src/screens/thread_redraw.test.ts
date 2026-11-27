import { afterEach, describe, expect, it } from "vitest";

import { OPEN_WORK } from "./messages";
import { waitingRedraw } from "./thread_redraw";

const OPEN = OPEN_WORK;

function setUp() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  let calls = 0;
  const redraw = waitingRedraw(host, async () => {
    calls++;
  }, OPEN);
  return { host, redraw, calls: () => calls };
}

/**
 * A redraw whose node answer waits until `release` is called, so a test can act meanwhile. Each
 * run draws a `.drawn` element that holds its number (the first run is 1). `release()` lets
 * every waiting run go on in the order they started; `release(1, 0)` lets them go on in the
 * order given, by run index.
 */
function setUpSlow() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  let calls = 0;
  const releases: Array<() => void> = [];
  const redraw = waitingRedraw(host, async (into) => {
    const run = ++calls;
    await new Promise<void>((resolve) => releases.push(resolve));
    const drawn = into.appendChild(document.createElement("p"));
    drawn.className = "drawn";
    drawn.textContent = String(run);
  }, OPEN);
  const release = async (...order: number[]) => {
    const waiting = releases.splice(0);
    (order.length > 0 ? order.map((i) => waiting[i]) : waiting).forEach((resolve) => resolve());
    await settle();
  };
  return { host, redraw, calls: () => calls, release };
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

  it("waits while settle-dialog or close-list is open and redraws when it leaves", async () => {
    const { host, redraw, calls } = setUp();
    const dialog = openElement(host, "settle-dialog");
    await redraw.later();
    await settle();
    expect(calls()).toBe(0);
    dialog.remove();
    await settle();
    expect(calls()).toBe(1);

    const closeList = openElement(host, "close-list");
    await redraw.later();
    await settle();
    expect(calls()).toBe(1);
    closeList.remove();
    await settle();
    expect(calls()).toBe(2);
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
    // The swap of `now` took the form away, and the observer then finds nothing waiting.
    expect(form.isConnected).toBe(false);
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

  it("keeps a form that is opened while the redraw waits for the node", async () => {
    const { host, redraw, calls, release } = setUpSlow();
    const running = redraw.later();
    const form = openElement(host, "quote-form");
    await release();
    await running;
    expect(host.querySelector(".quote-form")).toBe(form);
    expect(host.querySelector(".drawn")).toBeNull();
    expect(calls()).toBe(1);

    form.remove();
    await settle();
    await release();
    expect(calls()).toBe(2);
    expect(host.querySelector(".drawn")).not.toBeNull();
  });

  it("now draws at once over an open form, also when it waited for the node", async () => {
    const { host, redraw, release } = setUpSlow();
    openElement(host, "quote-form");
    const running = redraw.now();
    await release();
    await running;
    expect(host.querySelector(".quote-form")).toBeNull();
    expect(host.querySelector(".drawn")).not.toBeNull();
  });

  it("a later that is replaced by a newer now draws only the newer one", async () => {
    const { host, redraw, calls, release } = setUpSlow();
    const first = redraw.later();
    openElement(host, "quote-form");
    const second = redraw.now();
    // The older run ends last. If it swapped, the thread would show run 1.
    await release(1, 0);
    await Promise.all([first, second]);
    expect(calls()).toBe(2);
    expect(host.querySelectorAll(".drawn")).toHaveLength(1);
    expect(host.querySelector(".drawn")!.textContent).toBe("2");
    expect(host.querySelector(".quote-form")).toBeNull();
  });

  it("a now that a later redraw overtakes still draws at once over a form opened meanwhile", async () => {
    const { host, redraw, calls, release } = setUpSlow();
    const own = redraw.now();
    const feed = redraw.later();
    openElement(host, "quote-form");
    await release();
    await Promise.all([own, feed]);
    expect(calls()).toBe(2);
    expect(host.querySelector(".drawn")!.textContent).toBe("2");
    expect(host.querySelector(".quote-form")).toBeNull();
  });

  it("the debt of a now is paid by one swap only", async () => {
    const { host, redraw, calls, release } = setUpSlow();
    const own = redraw.now();
    await release();
    await own;
    expect(calls()).toBe(1);

    const feed = redraw.later();
    const form = openElement(host, "quote-form");
    await release();
    await feed;
    expect(host.querySelector(".quote-form")).toBe(form);
    expect(host.querySelector(".drawn")!.textContent).toBe("1");
  });
});
