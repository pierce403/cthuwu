import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { writeLeaderboardCache } from "./leaderboard-cache";
import { cachedSnapshot } from "./leaderboard-test-data";
import { LEADERBOARD_CACHE_KEY } from "./leaderboard-types";

const offlineScript = readFileSync(
  resolve(process.cwd(), "public/offline-leaderboard.js"),
  "utf8",
);

function renderOffline(): void {
  runInNewContext(offlineScript, {
    document,
    localStorage,
    location: { reload: vi.fn() },
  });
}

beforeEach(() => {
  localStorage.clear();
  document.body.innerHTML = `
    <p id="offline-source"></p>
    <ol id="offline-tentacles"></ol>
    <button id="offline-retry"></button>`;
});

describe("offline leaderboard shell", () => {
  it("renders the last validated local snapshot without a network request", () => {
    writeLeaderboardCache(localStorage, cachedSnapshot("Offline Tentacle"));
    renderOffline();
    expect(document.querySelector("#offline-source")?.textContent).toContain("Base 8453 · block 42000000");
    expect(document.querySelector("#offline-tentacles")?.textContent).toContain("Offline Tentacle");
    expect(document.querySelector("#offline-tentacles")?.textContent).toContain("Level 0.00");
  });

  it("removes only a corrupt leaderboard record", () => {
    localStorage.setItem(LEADERBOARD_CACHE_KEY, "{broken");
    localStorage.setItem("cthuwu:production:identity:v1", "preserve");
    renderOffline();
    expect(localStorage.getItem(LEADERBOARD_CACHE_KEY)).toBeNull();
    expect(localStorage.getItem("cthuwu:production:identity:v1")).toBe("preserve");
    expect(document.querySelector("#offline-source")?.textContent).toContain("No validated");
  });

  it("rejects a tampered ranked zero wallet instead of presenting it offline", () => {
    const snapshot = cachedSnapshot();
    snapshot.rankedWallets[0].wallet = "0x0000000000000000000000000000000000000000";
    snapshot.rankedWallets[0].identities[0].agentWallet =
      "0x0000000000000000000000000000000000000000";
    localStorage.setItem(LEADERBOARD_CACHE_KEY, JSON.stringify(snapshot));

    renderOffline();

    expect(document.querySelector("#offline-tentacles")?.textContent).toBe("");
    expect(localStorage.getItem(LEADERBOARD_CACHE_KEY)).toBeNull();
  });
});
