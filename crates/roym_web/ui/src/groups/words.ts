// Notice strings mirrored between roym_core::conversation::group and the Hub UI
// (Rust is the source of truth; a Rust-side test reads this file and compares
// the notice strings verbatim).

export const OWNER_CAN_READ_NOTICE =
  "The owner of this group makes and shares the group's key, so the owner can read every message sent while they own it. Adding or removing a member is shown to everyone in the group.";

export const GROUP_KEY_TRUST_NOTICE =
  "Each member's messages are signed with a key this installation first saw when they joined. That is a weaker check than a signed record, so messages here are never marked as verified.";

export const GROUP_DELIVERY_NOTICE =
  '"Not yet delivered to every member" means at least one member has not received it directly. Members also pass messages to each other, so a member may still receive it later.';

export const GROUP_JOIN_BOUNDARY_NOTICE =
  "You can read messages sent after you joined. Messages from before you joined are not shared with you.";

export const GROUP_REMOVED_NOTICE =
  "You were removed from this group. You can still read what you received before. You cannot read or send new messages.";

export const GROUP_RESTORED_NOTICE =
  "This group's history was restored from a backup. This installation is not a member, so it cannot send or receive new messages here. Ask the owner to add your new address.";

export const GROUP_ADD_UNREACHABLE_MESSAGE =
  "Could not reach this person to add them. Someone you have not talked to before must be online when you add them.";

export const GROUP_HIDDEN_NOTICE =
  "A hidden group is not shown, and its new messages are not kept here. This installation still receives them underneath, and you stay a member until the owner removes you.";

export const TRANSCRIPT_CHECK_NOTICE =
  "Members who see the same code hold the same messages in the same order. A member who blocked someone in the group sees a different code.";

export const CARDS_NOT_IN_GROUPS_MESSAGE =
  "Cards are sent only in a 1:1 conversation.";

export const SESSION_ENDED_NOTICE =
  "Your session ended, for example because this installation restarted. Log in again.";

export function deliveryWords(state: "pending" | "delivered" | "failed"): string {
  switch (state) {
    case "pending":
      return "Not yet delivered to every member";
    case "delivered":
      return "Delivered to every member";
    case "failed":
      return "Not delivered to every member";
  }
}

export interface MembershipEventPayload {
  action: "create" | "add" | "remove";
  subject: string;
  epoch: number;
}

export function membershipEventWords(
  event: MembershipEventPayload,
  nameOf: (addr: string) => string,
  author: string,
): string {
  const authorName = nameOf(author);
  const subjectName = nameOf(event.subject);
  switch (event.action) {
    case "create":
      return `${authorName} created the group`;
    case "add":
      return `${authorName} added ${subjectName}`;
    case "remove":
      return `${authorName} removed ${subjectName}`;
  }
}
