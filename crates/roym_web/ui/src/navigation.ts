export type TabKey =
  | "profile"
  | "contacts"
  | "messages"
  | "groups"
  | "listings"
  | "directory"
  | "memberships"
  | "synorg"
  | "safety"
  | "backup"
  | "components";

export interface RequestPreFill {
  listingId?: string;
  categories?: string[];
}

export interface MessagesInitialState {
  conversationId?: string;
  preFill?: RequestPreFill;
}

export type TabOpener = (tab: TabKey, initial?: unknown) => Promise<void>;

let activeOpener: TabOpener | null = null;

export function setTabOpener(opener: TabOpener | null): void {
  activeOpener = opener;
}

export async function openTab(tab: TabKey, initial?: unknown): Promise<void> {
  if (activeOpener) {
    await activeOpener(tab, initial);
  }
}
