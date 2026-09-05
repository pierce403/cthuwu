import type { ContentTypeId } from "@xmtp/content-type-primitives";
import { ListConversationsOrderBy } from "@xmtp/node-sdk";

const MAX_CATCH_UP_DMS = 256;
const MAX_CATCH_UP_MESSAGES_PER_DM = 512;

export interface CatchUpMessage {
  id: string;
  senderInboxId: string;
  sentAtNs: bigint;
  conversationId: string;
  contentType: ContentTypeId;
  content: unknown;
}

export interface CatchUpDm {
  messages(options: { limit: number }): Promise<CatchUpMessage[]>;
  send(content: unknown, options?: { shouldPush?: boolean }): Promise<unknown>;
  sendText(text: string): Promise<unknown>;
}

export interface CatchUpConversations {
  syncAll(): Promise<unknown>;
  listDms(options: { limit: number; orderBy: ListConversationsOrderBy }): CatchUpDm[];
}

export async function catchUpDirectMessages(options: {
  conversations: CatchUpConversations;
  selfInboxId: string;
  handle: (message: CatchUpMessage, conversation: CatchUpDm) => Promise<void>;
}): Promise<{ conversations: number; messages: number; truncated: boolean }> {
  await options.conversations.syncAll();
  const dms = options.conversations.listDms({
    limit: MAX_CATCH_UP_DMS,
    orderBy: ListConversationsOrderBy.LastActivity,
  });
  let messages = 0;
  let truncated = dms.length === MAX_CATCH_UP_DMS;

  for (const conversation of dms) {
    const recent = await conversation.messages({
      limit: MAX_CATCH_UP_MESSAGES_PER_DM,
    });
    if (recent.length === MAX_CATCH_UP_MESSAGES_PER_DM) {
      truncated = true;
    }
    recent.sort((left, right) => {
      if (left.sentAtNs === right.sentAtNs) {
        return left.id.localeCompare(right.id);
      }
      return left.sentAtNs < right.sentAtNs ? -1 : 1;
    });
    for (const message of recent) {
      if (message.senderInboxId === options.selfInboxId) {
        continue;
      }
      await options.handle(message, conversation);
      messages += 1;
    }
  }

  return { conversations: dms.length, messages, truncated };
}

/** Bounded group observation catch-up never sends old replies or enters the personal lane. */
export async function catchUpGroupMessages(options: {
  conversations: { syncAll(): Promise<unknown>; listGroups(options: { limit: number; orderBy: ListConversationsOrderBy }): CatchUpDm[] };
  selfInboxId: string;
  handle: (message: CatchUpMessage, conversation: CatchUpDm) => Promise<void>;
  now?: number;
}): Promise<void> {
  await options.conversations.syncAll();
  const cutoff = BigInt((options.now ?? Date.now()) - 14 * 86400 * 1000) * 1_000_000n;
  for (const group of options.conversations.listGroups({ limit: 64, orderBy: ListConversationsOrderBy.LastActivity })) {
    const messages = await group.messages({ limit: 128 });
    messages.sort((a, b) => a.sentAtNs < b.sentAtNs ? -1 : a.sentAtNs > b.sentAtNs ? 1 : a.id.localeCompare(b.id));
    for (const message of messages) {
      if (message.senderInboxId !== options.selfInboxId && message.sentAtNs >= cutoff) await options.handle(message, group);
    }
  }
}
