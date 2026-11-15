import { refiller } from "../dom";

/** The two ways to draw a chat thread again. */
export interface ThreadRedraw {
  /** Draw again, but wait while the person has open work in the thread. */
  later: () => Promise<void>;
  /** Draw again at once, and drop a redraw that waits. */
  now: () => Promise<void>;
}

/**
 * A redraw of `host` that does not throw away work the person has started.
 * `later` waits while an element that matches `openWork` is inside `host`,
 * and the check is made again just before the new content would replace the
 * old, because the person can open a form while the node is answering. The
 * waiting redraw runs once, when the last such element leaves. `now` is for
 * a redraw that the person asked for, such as after a send: it replaces the
 * content whatever is open.
 */
export function waitingRedraw(
  host: HTMLElement,
  fill: (into: DocumentFragment) => Promise<void>,
  openWork: string,
): ThreadRedraw {
  let waiting = false;
  // True when the newest run is a `now`. A run that is not the newest never
  // asks, so this is the kind of the run that asks.
  let atOnce = false;
  const hasOpenWork = () => host.querySelector(openWork) !== null;
  const draw = refiller(host, fill, () => {
    if (atOnce || !hasOpenWork()) return true;
    waiting = true;
    return false;
  });
  const later = async () => {
    if (hasOpenWork()) {
      waiting = true;
      return;
    }
    atOnce = false;
    await draw();
  };
  // When the thread is drawn anew, the old host leaves the page and no
  // mutation reaches this observer again, so it needs no disconnect.
  new MutationObserver(() => {
    if (waiting && !hasOpenWork()) {
      waiting = false;
      void later();
    }
  }).observe(host, { childList: true, subtree: true });
  return {
    later,
    now: async () => {
      waiting = false;
      atOnce = true;
      await draw();
    },
  };
}
