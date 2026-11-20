export interface RequestPreFill {
  listingId?: string;
  categories?: string[];
}

export interface MessagesInitialState {
  conversationId?: string;
  preFill?: RequestPreFill;
}

export interface TabStateMap {
  profile: void;
  contacts: void;
  messages: MessagesInitialState | undefined;
  groups: void;
  listings: void;
  directory: void;
  memberships: void;
  synorg: void;
  safety: void;
  backup: void;
  components: void;
}

export type TabKey = keyof TabStateMap;

export type TabOpener = <K extends TabKey>(tab: K, initial?: TabStateMap[K]) => Promise<void>;

let activeOpener: TabOpener | null = null;

export function setTabOpener(opener: TabOpener | null): void {
  activeOpener = opener;
}

export async function openTab<K extends TabKey>(tab: K, initial?: TabStateMap[K]): Promise<void> {
  if (activeOpener) {
    await activeOpener(tab, initial);
  }
}
