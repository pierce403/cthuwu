export const CHAT_CHANNELS = ["direct", "acolytes", "global"] as const;
export type ChatChannel = (typeof CHAT_CHANNELS)[number];

export type ChannelStatus =
  | "loading"
  | "awaiting-assignment"
  | "ready"
  | "empty"
  | "policy-blocked"
  | "error";

export interface WorkspaceMessage {
  id: string;
  conversationId: string;
  senderInboxId: string;
  sentAtNs: bigint;
  contentType: string;
  text: string;
  mine: boolean;
}

export interface ChannelBinding {
  readConversationIds: string[];
  writeConversationId?: string;
}

export interface ChannelSnapshot extends ChannelBinding {
  channel: ChatChannel;
  status: ChannelStatus;
  messages: WorkspaceMessage[];
  unread: number;
  hasMore: boolean;
  retentionVerified: boolean;
  typing: boolean;
  error?: string;
}

export type AssignmentState =
  | "checking"
  | "intro-unconfigured"
  | "intro-fallback"
  | "liveness-required"
  | "branding-active"
  | "anchor-verified"
  | "rotation-verified"
  | "liveness-unavailable"
  | "direct-verification-unavailable"
  | "registry-unavailable";

export interface WorkspaceSnapshot {
  inboxId: string;
  activeChannel: ChatChannel;
  connected: boolean;
  assignmentState: AssignmentState;
  assignmentNotice: string;
  tentacleName: string;
  assignedTentacleAddress?: string;
  channels: Record<ChatChannel, ChannelSnapshot>;
}

export interface ChatWorkspace {
  readonly inboxId: string;
  snapshot(): WorkspaceSnapshot;
  subscribe(listener: (snapshot: WorkspaceSnapshot) => void): () => void;
  setActiveChannel(channel: ChatChannel): void;
  setViewport(channel: ChatChannel, scrollTop: number, atBottom: boolean): void;
  savedScrollTop(channel: ChatChannel): number;
  loadEarlier(channel: ChatChannel): Promise<void>;
  send(channel: ChatChannel, text: string): Promise<void>;
  revalidateAssignment(reason: "connect" | "resume" | "periodic" | "retry"): Promise<void>;
  close(): Promise<void>;
}

export const RETENTION_FROM_NS = 1n;
export const RETENTION_IN_NS = 1_209_600_000_000_000n;
export const XMTP_GROUP_MEMBER_LIMIT = 250;
