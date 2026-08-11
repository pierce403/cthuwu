import { beforeEach, describe, expect, it } from "vitest";
import { CHAT_UI_STORAGE_KEY, createChatUiStateStore } from "./storage";

describe("chat tab state storage", () => {
  beforeEach(() => localStorage.clear());

  it("persists independent read and scroll state under cthuwu.chat.* only", () => {
    const store = createChatUiStateStore(localStorage);
    store.setActiveChannel("global");
    store.setReadAt("direct", 50n);
    store.setReadAt("acolytes", 80n);
    store.setScrollTop("global", 123.4);

    const restored = createChatUiStateStore(localStorage);
    expect(restored.activeChannel).toBe("global");
    expect(restored.lastReadAtNs("direct")).toBe(50n);
    expect(restored.lastReadAtNs("acolytes")).toBe(80n);
    expect(restored.scrollTop("global")).toBe(123);
    expect(CHAT_UI_STORAGE_KEY).toMatch(/^cthuwu\.chat\./u);
    expect([...Array(localStorage.length)].map((_, index) => localStorage.key(index))).toEqual([
      CHAT_UI_STORAGE_KEY,
    ]);
  });

  it("drops malformed state without touching unrelated cache keys", () => {
    localStorage.setItem(CHAT_UI_STORAGE_KEY, '{"version":1,"channels":{}}');
    localStorage.setItem("cthuwu.leaderboard.production.v1", "keep");
    const store = createChatUiStateStore(localStorage);
    expect(store.activeChannel).toBe("direct");
    expect(localStorage.getItem(CHAT_UI_STORAGE_KEY)).toBeNull();
    expect(localStorage.getItem("cthuwu.leaderboard.production.v1")).toBe("keep");
  });

  it("persists message-ID tie breakers at an equal read timestamp", () => {
    const store = createChatUiStateStore(localStorage);
    store.setReadAt("global", 100n, ["read-a"]);

    const restored = createChatUiStateStore(localStorage);
    expect(restored.readCursor("global")).toMatchObject({ sentAtNs: 100n });
    expect(restored.readCursor("global").messageIds).toEqual(new Set(["read-a"]));

    restored.setReadAt("global", 100n, ["read-b"]);
    const merged = createChatUiStateStore(localStorage).readCursor("global");
    expect(merged.sentAtNs).toBe(100n);
    expect(merged.messageIds).toEqual(new Set(["read-a", "read-b"]));
    expect(merged.messageIds.has("unseen-at-same-time")).toBe(false);
  });
});
