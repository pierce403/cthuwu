import { CHAT_CHANNELS, type ChatChannel } from "./types";

export const CHAT_UI_STORAGE_KEY = "cthuwu.chat.ui.production.v1";
const MAX_SCROLL_TOP = 10_000_000;
const MAX_READ_BOUNDARY_IDS = 1_000;
const MAX_STORED_STATE_BYTES = 256 * 1024;

interface StoredChannelState {
  lastReadAtNs: string;
  lastReadMessageIds: string[];
  scrollTop: number;
}

interface StoredChatUiState {
  version: 1;
  activeChannel: ChatChannel;
  channels: Record<ChatChannel, StoredChannelState>;
}

export interface ChatUiStateStore {
  activeChannel: ChatChannel;
  lastReadAtNs(channel: ChatChannel): bigint;
  readCursor(channel: ChatChannel): { sentAtNs: bigint; messageIds: ReadonlySet<string> };
  scrollTop(channel: ChatChannel): number;
  setActiveChannel(channel: ChatChannel): void;
  setReadAt(channel: ChatChannel, sentAtNs: bigint, messageIdsAtNs?: readonly string[]): void;
  setScrollTop(channel: ChatChannel, scrollTop: number): void;
}

export function createChatUiStateStore(storage: Storage | undefined): ChatUiStateStore {
  let state = read(storage);
  const persist = (): void => {
    if (!storage) return;
    try {
      storage.setItem(CHAT_UI_STORAGE_KEY, JSON.stringify(state));
    } catch {
      // Per-tab navigation state is best effort. XMTP remains the message source of truth.
    }
  };
  return {
    get activeChannel() {
      return state.activeChannel;
    },
    lastReadAtNs: (channel) => BigInt(state.channels[channel].lastReadAtNs),
    readCursor: (channel) => ({
      sentAtNs: BigInt(state.channels[channel].lastReadAtNs),
      messageIds: new Set(state.channels[channel].lastReadMessageIds),
    }),
    scrollTop: (channel) => state.channels[channel].scrollTop,
    setActiveChannel: (channel) => {
      state.activeChannel = channel;
      persist();
    },
    setReadAt: (channel, sentAtNs, messageIdsAtNs = []) => {
      const current = state.channels[channel];
      const messageIds = validMessageIds(messageIdsAtNs);
      if (sentAtNs > BigInt(current.lastReadAtNs)) {
        state.channels[channel].lastReadAtNs = sentAtNs.toString();
        state.channels[channel].lastReadMessageIds = messageIds;
        persist();
      } else if (sentAtNs === BigInt(current.lastReadAtNs)) {
        const merged = validMessageIds([...current.lastReadMessageIds, ...messageIds]);
        if (merged.length !== current.lastReadMessageIds.length) {
          current.lastReadMessageIds = merged;
          persist();
        }
      }
    },
    setScrollTop: (channel, scrollTop) => {
      state.channels[channel].scrollTop = clampScroll(scrollTop);
      persist();
    },
  };
}

function read(storage: Storage | undefined): StoredChatUiState {
  if (!storage) return emptyState();
  try {
    const raw = storage.getItem(CHAT_UI_STORAGE_KEY);
    if (!raw || new TextEncoder().encode(raw).length > MAX_STORED_STATE_BYTES) return emptyState();
    const parsed = JSON.parse(raw) as unknown;
    if (!isRecord(parsed) || parsed.version !== 1 || !isChannel(parsed.activeChannel)) {
      throw new Error();
    }
    if (!isRecord(parsed.channels)) throw new Error();
    const parsedChannels = parsed.channels;
    const channels = Object.fromEntries(
      CHAT_CHANNELS.map((channel) => {
        const candidate = parsedChannels[channel];
        if (
          !isRecord(candidate) ||
          typeof candidate.lastReadAtNs !== "string" ||
          !/^(0|[1-9][0-9]{0,24})$/u.test(candidate.lastReadAtNs) ||
          (candidate.lastReadMessageIds !== undefined &&
            !Array.isArray(candidate.lastReadMessageIds)) ||
          typeof candidate.scrollTop !== "number" ||
          !Number.isFinite(candidate.scrollTop)
        ) {
          throw new Error();
        }
        return [
          channel,
          {
            lastReadAtNs: candidate.lastReadAtNs,
            lastReadMessageIds: validMessageIds(
              Array.isArray(candidate.lastReadMessageIds) ? candidate.lastReadMessageIds : [],
            ),
            scrollTop: clampScroll(candidate.scrollTop),
          },
        ];
      }),
    ) as Record<ChatChannel, StoredChannelState>;
    return { version: 1, activeChannel: parsed.activeChannel, channels };
  } catch {
    try {
      storage.removeItem(CHAT_UI_STORAGE_KEY);
    } catch {
      // Ignore an unavailable storage area.
    }
    return emptyState();
  }
}

function emptyState(): StoredChatUiState {
  return {
    version: 1,
    activeChannel: "direct",
    channels: {
      direct: { lastReadAtNs: "0", lastReadMessageIds: [], scrollTop: 0 },
      acolytes: { lastReadAtNs: "0", lastReadMessageIds: [], scrollTop: 0 },
      global: { lastReadAtNs: "0", lastReadMessageIds: [], scrollTop: 0 },
    },
  };
}

function validMessageIds(value: readonly unknown[]): string[] {
  const ids = [...new Set(value)].map((id) => {
    if (
      typeof id !== "string" || id.length === 0 || id.length > 256 ||
      [...id].some((character) => (character.codePointAt(0) ?? 0) < 0x20)
    ) {
      throw new Error("read boundary message ID is invalid");
    }
    return id;
  });
  if (ids.length > MAX_READ_BOUNDARY_IDS) throw new Error("read boundary is too large");
  return ids.sort();
}

function clampScroll(value: number): number {
  return Math.min(MAX_SCROLL_TOP, Math.max(0, Math.round(value)));
}

function isChannel(value: unknown): value is ChatChannel {
  return typeof value === "string" && (CHAT_CHANNELS as readonly string[]).includes(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
