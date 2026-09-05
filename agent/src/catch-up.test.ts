import { describe, expect, it, vi } from "vitest";
import { contentTypeText } from "@xmtp/node-sdk";
import { catchUpDirectMessages, catchUpGroupMessages, type CatchUpMessage } from "./catch-up.js";

function message(id: string, senderInboxId: string, sentAtNs: bigint): CatchUpMessage {
  return {
    id,
    senderInboxId,
    sentAtNs,
    conversationId: "dm-1",
    contentType: contentTypeText(),
    content: id,
  };
}

describe("startup DM catch-up", () => {
  it("syncs first, skips self messages, and replays inbound history oldest first", async () => {
    const order: string[] = [];
    const syncAll = vi.fn(async () => {
      order.push("sync");
    });
    const conversation = {
      messages: vi.fn(async () => [
        message("new", "acolyte", 3n),
        message("reply", "tentacle", 2n),
        message("old", "acolyte", 1n),
      ]),
      send: vi.fn(async () => undefined),
      sendText: vi.fn(async () => undefined),
    };
    const listDms = vi.fn(() => [conversation]);

    const result = await catchUpDirectMessages({
      conversations: { syncAll, listDms },
      selfInboxId: "tentacle",
      handle: async (entry) => {
        order.push(entry.id);
      },
    });

    expect(order).toEqual(["sync", "old", "new"]);
    expect(result).toEqual({ conversations: 1, messages: 2, truncated: false });
    expect(listDms).toHaveBeenCalledWith({ limit: 256, orderBy: 1 });
    expect(conversation.messages).toHaveBeenCalledWith({ limit: 512 });
  });
});

it("group catch-up observes recent inbound text chronologically without sending", async () => {
  const now = 20 * 86400 * 1000;
  const recent = BigInt(now) * 1_000_000n;
  const group = { messages: vi.fn(async () => [message("latest", "acolyte", recent), message("old", "acolyte", 1n), message("self", "tentacle", recent), message("earlier", "acolyte", recent - 1n)]), send: vi.fn(), sendText: vi.fn() };
  const handle = vi.fn();
  await catchUpGroupMessages({ conversations: { syncAll: vi.fn(), listGroups: vi.fn(() => [group]) }, selfInboxId: "tentacle", handle, now });
  expect(handle.mock.calls.map(([m]) => m.id)).toEqual(["earlier", "latest"]);
  expect(group.sendText).not.toHaveBeenCalled();
});
