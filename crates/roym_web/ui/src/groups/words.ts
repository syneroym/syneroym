// Group content types, and the words the Hub builds from a delivery state or a
// membership event. The notice sentences are in the string catalog; the ones
// that `roym_core::conversation::group` also holds are checked against
// `i18n/en.ftl` by a Rust test.

import { t } from "../i18n/i18n";

export const GROUP_PROFILE_CONTENT_TYPE = "application/vnd.roym.group-profile+json";
export const MEMBERSHIP_EVENT_CONTENT_TYPE = "application/vnd.roym.membership-event+json";

export function deliveryWords(state: "pending" | "delivered" | "failed"): string {
  switch (state) {
    case "pending":
      return t("group-delivery-pending");
    case "delivered":
      return t("group-delivery-delivered");
    case "failed":
      return t("group-delivery-failed");
  }
}

export function shortAddress(addr: string): string {
  if (!addr) return "";
  if (addr.length <= 16) return addr;
  return addr.slice(0, 12) + "…";
}

export interface MembershipEventPayload {
  action: "create" | "add" | "remove" | string;
  subject: string;
  epoch?: number;
  author?: string;
}

export function membershipEventWords(
  event: MembershipEventPayload,
  nameOf: (addr: string) => string,
  author?: string,
): string {
  const authorAddr = author || event.author || "";
  const authorName = nameOf(authorAddr);
  const subjectName = nameOf(event.subject);
  if (event.action === "create" || (event.action === "add" && authorAddr && event.subject === authorAddr)) {
    return t("group-event-created", { author: authorName });
  }
  switch (event.action) {
    case "add":
      return t("group-event-added", { author: authorName, subject: subjectName });
    case "remove":
      return t("group-event-removed", { author: authorName, subject: subjectName });
    default:
      return t("group-event-updated", { author: authorName });
  }
}

