import { setEventFeedCookie, storedToken } from "./session/login";

export type FeedChangeEvent = {
  kind: string;
  subject: string;
  item: string | null;
  kinds?: string[];
};

export type FeedReloadEvent = {
  reload: true;
};

export type FeedEvent = FeedChangeEvent | FeedReloadEvent;

export type FeedListener = (event: FeedEvent) => void;

let lastCursor: string | null = null;
let currentEs: EventSource | null = null;
let backoffMs = 1000;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let feedStarted = false;
let visibilityListenerInstalled = false;

const listeners = new Set<FeedListener>();
type PendingBurst = {
  timer: ReturnType<typeof setTimeout>;
  lastEvent: FeedChangeEvent;
  kinds: Set<string>;
};
const pendingBySubject = new Map<string, PendingBurst>();

export function onFeed(listener: FeedListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function extractCursor(e: MessageEvent): string | null {
  if (e.lastEventId) {
    return e.lastEventId;
  }
  try {
    const parsed = JSON.parse(e.data) as { cursor?: string };
    if (parsed.cursor) {
      return parsed.cursor;
    }
  } catch {
    /* non-json body */
  }
  return null;
}

export function openFeed(): void {
  if (typeof document !== "undefined" && document.visibilityState !== "visible") {
    return;
  }
  if (currentEs) {
    return;
  }
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  if (typeof document !== "undefined") {
    const token = storedToken();
    if (token) {
      setEventFeedCookie(token);
    }
  }

  const url = lastCursor
    ? `/_syneroym/events?after=${encodeURIComponent(lastCursor)}`
    : "/_syneroym/events";

  const es = new EventSource(url);
  currentEs = es;

  es.addEventListener("ready", (e: MessageEvent) => {
    const cur = extractCursor(e);
    if (cur) {
      lastCursor = cur;
    }
    backoffMs = 1000;
    clearPendingChanges();
    notifyListeners({ reload: true });
  });

  es.addEventListener("reload", (e: MessageEvent) => {
    const cur = extractCursor(e);
    if (cur) {
      lastCursor = cur;
    }
    backoffMs = 1000;
    clearPendingChanges();
    notifyListeners({ reload: true });
  });

  es.addEventListener("change", (e: MessageEvent) => {
    const cur = extractCursor(e);
    if (cur) {
      lastCursor = cur;
    }
    backoffMs = 1000;

    try {
      const data = JSON.parse(e.data) as {
        kind: string;
        subject: string;
        item?: string | null;
      };
      const changeEvent: FeedChangeEvent = {
        kind: data.kind,
        subject: data.subject,
        item: data.item ?? null,
      };

      const existing = pendingBySubject.get(data.subject);
      const kinds = existing ? existing.kinds : new Set<string>();
      kinds.add(data.kind);
      if (existing) {
        clearTimeout(existing.timer);
      }
      const timer = setTimeout(() => {
        const burst = pendingBySubject.get(data.subject);
        pendingBySubject.delete(data.subject);
        if (burst) {
          notifyListeners({
            ...burst.lastEvent,
            kinds: Array.from(burst.kinds),
          });
        }
      }, 150);
      pendingBySubject.set(data.subject, {
        timer,
        lastEvent: changeEvent,
        kinds,
      });
    } catch (err) {
      console.error("malformed event data", err);
    }
  });

  es.onerror = () => {
    if (es.readyState === EventSource.CLOSED) {
      closeFeed();
      if (typeof document !== "undefined" && document.visibilityState === "visible") {
        const jitter = Math.random() * 0.2;
        const delay = Math.round(backoffMs * (1 + jitter));
        backoffMs = Math.min(30000, backoffMs * 2);
        reconnectTimer = setTimeout(() => {
          reconnectTimer = null;
          openFeed();
        }, delay);
      }
    }
  };
}

export function closeFeed(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (currentEs) {
    currentEs.close();
    currentEs = null;
  }
}

export function startFeed(): void {
  if (feedStarted) return;
  feedStarted = true;

  if (!visibilityListenerInstalled && typeof document !== "undefined") {
    visibilityListenerInstalled = true;
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") {
        openFeed();
      } else {
        closeFeed();
      }
    });
  }

  if (typeof document !== "undefined" && document.visibilityState === "visible") {
    openFeed();
  }
}

function clearPendingChanges(): void {
  for (const burst of pendingBySubject.values()) {
    clearTimeout(burst.timer);
  }
  pendingBySubject.clear();
}

function notifyListeners(event: FeedEvent): void {
  for (const listener of Array.from(listeners)) {
    try {
      listener(event);
    } catch (err) {
      console.error("feed listener threw", err);
    }
  }
}

export function _resetForTest(): void {
  closeFeed();
  clearPendingChanges();
  listeners.clear();
  lastCursor = null;
  backoffMs = 1000;
  feedStarted = false;
  visibilityListenerInstalled = false;
}

export function _getLastCursorForTest(): string | null {
  return lastCursor;
}

export function _getBackoffForTest(): number {
  return backoffMs;
}

export function _getCurrentEventSourceForTest(): EventSource | null {
  return currentEs;
}
