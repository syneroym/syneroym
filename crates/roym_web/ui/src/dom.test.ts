import { describe, expect, it } from "vitest";
import { refiller } from "./dom";

/// A promise the test settles by hand, so it controls which run finishes first.
function gate() {
  let open!: () => void;
  const wait = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { wait, open };
}

function row(label: string): HTMLElement {
  const el = document.createElement("div");
  el.textContent = label;
  return el;
}

describe("refiller", () => {
  it("draws one copy when two runs overlap", async () => {
    const host = document.createElement("div");
    const gates = [gate(), gate()];
    let started = 0;
    const reload = refiller(host, async (into) => {
      const mine = started++;
      await gates[mine].wait;
      into.appendChild(row(`run ${mine}`));
      return mine;
    });

    const first = reload();
    const second = reload();
    gates[1].open();
    gates[0].open();

    expect(await second).toBe(1);
    expect(await first).toBeUndefined();
    expect(host.children).toHaveLength(1);
    expect(host.textContent).toBe("run 1");
  });

  it("keeps the newest run even when an older one finishes last", async () => {
    const host = document.createElement("div");
    const gates = [gate(), gate()];
    let started = 0;
    const reload = refiller(host, async (into) => {
      const mine = started++;
      await gates[mine].wait;
      into.appendChild(row(`run ${mine}`));
    });

    const first = reload();
    const second = reload();
    gates[0].open();
    await first;
    expect(host.children).toHaveLength(0);
    gates[1].open();
    await second;
    expect(host.textContent).toBe("run 1");
  });

  it("replaces the old rows on each run and passes arguments through", async () => {
    const host = document.createElement("div");
    host.appendChild(row("stale"));
    const reload = refiller(host, async (into, label: string) => {
      into.appendChild(row(label));
    });

    await reload("a");
    await reload("b");
    expect(host.children).toHaveLength(1);
    expect(host.textContent).toBe("b");
  });
});
