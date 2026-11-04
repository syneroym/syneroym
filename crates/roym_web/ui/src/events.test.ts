import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  _getCurrentEventSourceForTest,
  _getLastCursorForTest,
  _resetForTest,
  onFeed,
  openFeed,
  startFeed,
} from "./events";

type EventListenerFn = (e: MessageEvent) => void;

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  url: string;
  readyState = FakeEventSource.OPEN;
  listeners = new Map<string, Set<EventListenerFn>>();
  onerror: ((e: Event) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, fn: EventListenerFn): void {
    if (!this.listeners.has(type)) {
      this.listeners.set(type, new Set());
    }
    this.listeners.get(type)?.add(fn);
  }

  removeEventListener(type: string, fn: EventListenerFn): void {
    this.listeners.get(type)?.delete(fn);
  }

  emit(type: string, data: string, lastEventId = ""): void {
    const event = {
      data,
      lastEventId,
    } as MessageEvent;
    for (const fn of this.listeners.get(type) || []) {
      fn(event);
    }
  }

  close(): void {
    this.readyState = FakeEventSource.CLOSED;
  }

  simulateErrorClosed(): void {
    this.readyState = FakeEventSource.CLOSED;
    this.onerror?.(new Event("error"));
  }
}

describe("Event feed", () => {
  let visibilityState = "visible";

  beforeEach(() => {
    vi.useFakeTimers();
    FakeEventSource.instances = [];
    visibilityState = "visible";

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibilityState,
    });

    vi.stubGlobal("EventSource", FakeEventSource);
    _resetForTest();
  });

  afterEach(() => {
    _resetForTest();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("a_burst_for_one_subject_gives_one_call", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;
    expect(es).toBeDefined();

    const calls: unknown[] = [];
    onFeed((ev) => calls.push(ev));

    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "m1" }), "cur-1");
    vi.advanceTimersByTime(50);
    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "m2" }), "cur-2");
    vi.advanceTimersByTime(50);
    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "m3" }), "cur-3");

    expect(calls.length).toBe(0);

    vi.advanceTimersByTime(150);
    expect(calls.length).toBe(1);
    expect(calls[0]).toEqual({
      kind: "conversation.message",
      subject: "conv-1",
      item: "m3",
      kinds: ["conversation.message"],
    });
  });

  it("a_burst_with_multiple_kinds_preserves_all_seen_kinds", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;
    expect(es).toBeDefined();

    const calls: unknown[] = [];
    onFeed((ev) => calls.push(ev));

    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "m1" }), "cur-1");
    vi.advanceTimersByTime(50);
    es.emit("change", JSON.stringify({ kind: "transaction.card", subject: "conv-1", item: "c1" }), "cur-2");

    vi.advanceTimersByTime(150);
    expect(calls.length).toBe(1);
    expect(calls[0]).toEqual({
      kind: "transaction.card",
      subject: "conv-1",
      item: "c1",
      kinds: ["conversation.message", "transaction.card"],
    });
  });

  it("a_listener_whose_screen_is_gone_is_removed", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;

    let called = false;
    const unsubscribe = onFeed(() => {
      called = true;
    });

    unsubscribe();

    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "conv-1", item: "m1" }), "cur-1");
    vi.advanceTimersByTime(200);

    expect(called).toBe(false);
  });

  it("reload_reaches_every_listener", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;

    let callCount1 = 0;
    let callCount2 = 0;
    onFeed((e) => {
      if ("reload" in e && e.reload) callCount1++;
    });
    onFeed((e) => {
      if ("reload" in e && e.reload) callCount2++;
    });

    es.emit("reload", JSON.stringify({ cursor: "now-1" }), "now-1");

    expect(callCount1).toBe(1);
    expect(callCount2).toBe(1);
    expect(_getLastCursorForTest()).toBe("now-1");
  });

  it("a_ready_frame_refreshes_every_listener_once", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;

    let readyCount = 0;
    onFeed((e) => {
      if ("reload" in e && e.reload) readyCount++;
    });

    es.emit("ready", JSON.stringify({ cursor: "head-1" }), "head-1");

    expect(readyCount).toBe(1);
    expect(_getLastCursorForTest()).toBe("head-1");
  });

  it("changes_for_another_subject_do_not_refresh_the_open_thread", () => {
    startFeed();
    const es = _getCurrentEventSourceForTest() as unknown as FakeEventSource;

    const openThreadId = "thread-open";
    let refreshedOpenThread = false;

    onFeed((e) => {
      if ("kind" in e && e.subject === openThreadId) {
        refreshedOpenThread = true;
      }
    });

    es.emit("change", JSON.stringify({ kind: "conversation.message", subject: "thread-other", item: "m1" }), "c-1");
    vi.advanceTimersByTime(200);

    expect(refreshedOpenThread).toBe(false);
  });

  it("a_closed_feed_is_opened_again_with_the_last_cursor", () => {
    startFeed();
    expect(FakeEventSource.instances.length).toBe(1);
    const es1 = FakeEventSource.instances[0];
    expect(es1.url).toBe("/_syneroym/events");

    es1.emit("change", JSON.stringify({ kind: "conversation.message", subject: "c1", item: "m1" }), "last-cur-1");
    expect(_getLastCursorForTest()).toBe("last-cur-1");

    es1.simulateErrorClosed();
    expect(_getCurrentEventSourceForTest()).toBeNull();

    // Initial backoff is 1s + jitter up to 20% (1000 - 1200ms)
    vi.advanceTimersByTime(1300);

    expect(FakeEventSource.instances.length).toBe(2);
    const es2 = FakeEventSource.instances[1];
    expect(es2.url).toBe("/_syneroym/events?after=last-cur-1");
  });

  it("a_hidden_tab_closes_its_feed_and_reopens_from_the_last_cursor_when_shown", () => {
    startFeed();
    expect(FakeEventSource.instances.length).toBe(1);
    const es1 = FakeEventSource.instances[0];

    es1.emit("change", JSON.stringify({ kind: "conversation.message", subject: "c1", item: "m1" }), "cur-x");

    // Hide tab
    visibilityState = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));

    expect(_getCurrentEventSourceForTest()).toBeNull();
    expect(es1.readyState).toBe(FakeEventSource.CLOSED);

    // Show tab
    visibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));

    expect(FakeEventSource.instances.length).toBe(2);
    const es2 = FakeEventSource.instances[1];
    expect(es2.url).toBe("/_syneroym/events?after=cur-x");
  });

  it("a_feed_never_opens_while_the_tab_is_hidden", () => {
    visibilityState = "hidden";
    startFeed();

    expect(_getCurrentEventSourceForTest()).toBeNull();
    expect(FakeEventSource.instances.length).toBe(0);

    openFeed();
    expect(_getCurrentEventSourceForTest()).toBeNull();
    expect(FakeEventSource.instances.length).toBe(0);
  });
});
