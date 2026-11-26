import { codeWord, messageId } from "../i18n/i18n";
import type { StoredState } from "../widgets/view";

export const STATE_LABELS = new Map<string, string>([
  ["pending", messageId("messages-state-pending")],
  ["deposited", messageId("messages-state-deposited")],
  ["delivered", messageId("messages-state-delivered")],
  ["failed", messageId("messages-state-failed")],
]);

export function messageStateWord(state: StoredState | string): string {
  return codeWord(STATE_LABELS, state);
}

export function canRetry(m: {
  direction?: string;
  state: StoredState | string;
  restored?: boolean;
  deleted_at_secs?: number | null;
}): boolean {
  return m.state === "failed" && m.deleted_at_secs == null && !m.restored;
}
