/** A stand-in for the browser's `EventSource` that a test drives with `emit`. */
export class FakeFeedSource {
  static instance: FakeFeedSource | null = null;
  static readonly OPEN = 1;
  readyState = 1;
  listeners = new Map<string, Set<(e: { data: string; lastEventId: string }) => void>>();

  constructor() {
    FakeFeedSource.instance = this;
  }

  addEventListener(type: string, fn: (e: { data: string; lastEventId: string }) => void) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)!.add(fn);
  }

  emit(type: string, data: string, lastEventId = "") {
    for (const fn of this.listeners.get(type) || []) {
      fn({ data, lastEventId });
    }
  }

  close() {}
}
