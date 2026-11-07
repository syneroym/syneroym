// Words the Hub shows for the signed place and audience values, so a
// person never reads the raw value.
//
// A place is named from the giver of the work (the quote author, or the
// listing owner) and the receiver of the work (the other person).

import { codeWord, messageId } from "../i18n/i18n";
import type { OpenTo, ServiceLocationKind } from "./editor";

/// For the forms where the person is the one who gives the work.
const EDITOR_LABELS = new Map<string, string>([
  ["at-giver", messageId("place-editor-at-giver")],
  ["at-receiver", messageId("place-editor-at-receiver")],
  ["remote", messageId("place-remote")],
]);

/// For cards, which both people read.
const CARD_LABELS = new Map<string, string>([
  ["at-giver", messageId("place-card-at-giver")],
  ["at-receiver", messageId("place-card-at-receiver")],
  ["remote", messageId("place-remote")],
]);

const OPEN_TO_LABELS = new Map<string, string>([
  ["anyone", messageId("audience-anyone")],
  ["members", messageId("audience-members")],
  ["referral", messageId("audience-referral")],
  ["past-counterparties", messageId("audience-past-counterparties")],
]);

export function serviceLocationEditorLabel(kind: ServiceLocationKind): string {
  return codeWord(EDITOR_LABELS, kind);
}

/// A card is data from another node, so an unknown value is shown as it
/// came, never as "undefined".
export function serviceLocationCardLabel(where: string): string {
  return codeWord(CARD_LABELS, where);
}

/// The order the listing editor offers the places in.
export const SERVICE_LOCATION_KINDS: ServiceLocationKind[] = ["remote", "at-giver", "at-receiver"];

/// The order the listing editor offers the audiences in.
export const OPEN_TO_KINDS: OpenTo[] = ["anyone", "members", "referral", "past-counterparties"];

export function openToLabel(kind: OpenTo): string {
  return codeWord(OPEN_TO_LABELS, kind);
}
