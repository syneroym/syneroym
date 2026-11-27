import { text } from "../dom";
import { formatUtcDate } from "../i18n/format";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import type { ForwardMode } from "./forward_mode";
import { openForwardPicker } from "./forward_picker";

export interface OriginView {
  author_did: string;
  written_at_secs: number;
  record_id: string;
  /** The author is this reader, so the Hub says "you". */
  is_you: boolean;
}

export interface ForwardView {
  mode: ForwardMode;
  forwardable: boolean;
  origin?: OriginView;
  origin_error?: string;
  forward_count: number;
  many_forwards: boolean;
  forwarded: boolean;
}

export interface ForwardableRow {
  id?: string;
  content_type?: string;
  deleted_at_secs?: number;
  forward?: ForwardView;
}

const CHAT_CONTENT_TYPE = "application/vnd.roym.message+json";
/** Marks where the author's name element goes inside a translated line. */
const AUTHOR_MARKER = "\u0001";

function shortDid(did: string): string {
  return `…${did.slice(-12)}`;
}

function authorName(origin: OriginView, contacts?: Map<string, string>): HTMLElement {
  const name = document.createElement("span");
  name.className = "forward-author";
  if (origin.is_you) {
    name.textContent = t("forward-author-you");
    return name;
  }
  if (contacts?.has(origin.author_did)) {
    const known = contacts.get(origin.author_did)?.trim();
    name.textContent = known ? known : shortDid(origin.author_did);
    name.title = origin.author_did;
    return name;
  }
  name.textContent = t("forward-not-in-contacts", {
    short: origin.author_did.slice(-12),
  });
  name.title = origin.author_did;
  return name;
}

/**
 * "Forwarded. First written by … on …. Forwarded N times." The name comes
 * only from the reader's contacts, or is "you"; never from the message.
 */
export function forwardLine(
  view: ForwardView | undefined,
  contacts?: Map<string, string>,
): HTMLElement | null {
  if (!view?.forwarded || !view.origin) return null;
  const origin = view.origin;
  const line = document.createElement("div");
  line.className = "forward-line";

  const template = view.many_forwards
    ? t("forward-line-many", {
        author: AUTHOR_MARKER,
        date: formatUtcDate(origin.written_at_secs),
        count: view.forward_count,
      })
    : t("forward-line", {
        author: AUTHOR_MARKER,
        date: formatUtcDate(origin.written_at_secs),
        count: view.forward_count,
      });
  const [before, ...after] = template.split(AUTHOR_MARKER);
  line.append(before);
  if (after.length > 0) {
    line.appendChild(authorName(origin, contacts));
    line.append(after.join(AUTHOR_MARKER));
  }
  return line;
}

/** The small tag that says why a row has no forward action, or "Signed". */
export function forwardTag(row: ForwardableRow): HTMLElement | null {
  if (row.deleted_at_secs != null || !row.forward) return null;
  const view = row.forward;
  if (view.mode === "not-forwardable") {
    // An unreadable body is not the author's choice; the body note says so.
    if (view.origin_error) return null;
    return text("span", t("forward-tag-not-forwardable"), "forward-tag not-forwardable");
  }
  if (view.forwardable) return text("span", t("forward-signed-tag"), "forward-tag signed");
  return null;
}

/** The note for a signed body whose origin did not check. */
export function forwardNote(view: ForwardView | undefined): HTMLElement | null {
  if (!view || view.mode !== "signed" || view.forwardable) return null;
  const note = text("span", t("forward-origin-not-checked"), "forward-origin-not-checked");
  if (view.origin_error) note.title = view.origin_error;
  return note;
}

/** The "Forward" action, offered only on a checked and signed message. */
export function forwardActions(
  row: ForwardableRow,
  refresh: () => Promise<void>,
): HTMLElement | null {
  if (!row.forward?.forwardable || !row.id) return null;
  const messageId = row.id;
  const box = document.createElement("div");
  box.className = "forward-actions";
  const button = text("button", t("forward-button"), "button forward-button") as HTMLButtonElement;
  button.onclick = () =>
    void openForwardPicker(button, async (conversationId) => {
      await call("conversation.forward", {
        message_id: messageId,
        conversation: conversationId,
      });
      await refresh();
    });
  box.appendChild(button);
  return box;
}

/** The tag for a plain message: it has no origin, so it cannot be forwarded. */
export function plainTag(row: ForwardableRow): HTMLElement | null {
  if (row.deleted_at_secs != null) return null;
  if (row.content_type !== "text/plain") return null;
  return text("span", t("forward-tag-no-origin"), "forward-tag no-origin");
}

/** `text/plain` rows and chat bodies, for the tag and the note. */
export function isChatOrPlain(row: ForwardableRow): boolean {
  return row.content_type === "text/plain" || row.content_type === CHAT_CONTENT_TYPE;
}
