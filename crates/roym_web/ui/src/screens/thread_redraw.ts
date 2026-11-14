/** The two ways to draw a chat thread again. */
export interface ThreadRedraw {
  /** Draw again, but wait while the person has open work in the thread. */
  later: () => Promise<void>;
  /** Draw again at once, and drop a redraw that waits. */
  now: () => Promise<void>;
}

/**
 * A redraw of `host` that does not throw away work the person has started.
 * `later` waits while an element that matches `openWork` is inside `host`. The
 * waiting redraw runs once, when the last such element leaves. `now` is for a
 * redraw that the person asked for, such as after a send.
 */
export function waitingRedraw(
  host: HTMLElement,
  redraw: () => Promise<void>,
  openWork: string,
): ThreadRedraw {
  let waiting = false;
  // When the thread is drawn anew, the old host leaves the page and no
  // mutation reaches this observer again, so it needs no disconnect.
  new MutationObserver(() => {
    if (waiting && host.querySelector(openWork) === null) {
      waiting = false;
      void redraw();
    }
  }).observe(host, { childList: true, subtree: true });
  return {
    later: async () => {
      if (host.querySelector(openWork) !== null) {
        waiting = true;
        return;
      }
      await redraw();
    },
    now: async () => {
      waiting = false;
      await redraw();
    },
  };
}
