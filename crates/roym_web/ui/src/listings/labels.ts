// Words the Hub shows for the signed place and audience values, so a
// person never reads the raw value.
//
// A place is named from the giver of the work (the quote author, or the
// listing owner) and the receiver of the work (the other person).

import type { OpenTo, ServiceLocationKind } from "./editor";

/// For the forms where the person is the one who gives the work.
export const SERVICE_LOCATION_EDITOR_LABELS: Record<ServiceLocationKind, string> = {
  "at-giver": "At my place",
  "at-receiver": "At their place",
  remote: "Remote",
};

/// For cards, which both people read.
export const SERVICE_LOCATION_CARD_LABELS: Record<ServiceLocationKind, string> = {
  "at-giver": "At the quote author's place",
  "at-receiver": "At the counterparty's place",
  remote: "Remote",
};

/// A card is data from another node, so an unknown value is shown as it
/// came, never as "undefined".
export function serviceLocationCardLabel(where: string): string {
  return (SERVICE_LOCATION_CARD_LABELS as Record<string, string>)[where] ?? where;
}

/// The order the listing editor offers the places in.
export const SERVICE_LOCATION_KINDS: ServiceLocationKind[] = ["remote", "at-giver", "at-receiver"];

/// The order the listing editor offers the audiences in.
export const OPEN_TO_KINDS: OpenTo[] = ["anyone", "members", "referral", "past-counterparties"];

export const OPEN_TO_LABELS: Record<OpenTo, string> = {
  anyone: "anyone",
  members: "members",
  referral: "referral",
  "past-counterparties": "People I have worked with before",
};
