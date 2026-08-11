import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { readLeaderboardCache, writeLeaderboardCache } from "./leaderboard-cache";
import { initializeLeaderboard, type LeaderboardElements } from "./leaderboard";
import { cachedSnapshot } from "./leaderboard-test-data";
import { ZERO_ADDRESS } from "./leaderboard-types";

function mountElements(): LeaderboardElements {
  document.body.innerHTML = `
    <section id="root">
      <span id="state"></span><span id="source"></span><span id="summary"></span>
      <button id="refresh"></button>
      <input id="search"><select id="funding"><option value="all">all</option><option value="funded">funded</option><option value="unfunded">unfunded</option></select>
      <select id="verification"><option value="all">all</option><option value="verified">verified</option><option value="suspended">suspended</option></select>
      <select id="protocol"><option value="all">all</option><option value="v1">v1</option><option value="other">other</option></select>
      <select id="shared"><option value="all">all</option><option value="unique">unique</option><option value="shared">shared</option></select>
      <select id="sort"><option value="rank">rank</option><option value="name">name</option></select>
      <div id="ranked"></div><section id="suspended-section"><div id="suspended"></div></section>
      <p id="empty"></p>
    </section>`;
  const get = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;
  return {
    root: get("root"),
    status: get("state"),
    source: get("source"),
    summary: get("summary"),
    refresh: get("refresh"),
    search: get("search"),
    funding: get("funding"),
    verification: get("verification"),
    protocol: get("protocol"),
    shared: get("shared"),
    sort: get("sort"),
    ranked: get("ranked"),
    suspendedSection: get("suspended-section"),
    suspended: get("suspended"),
    empty: get("empty"),
  };
}

const config = {
  cacheFreshnessMs: 900_000,
  ipfsGateway: "https://cloudflare-ipfs.com/ipfs/",
  arweaveGateway: "https://arweave.net/",
};

beforeEach(() => localStorage.clear());

describe("Tentacle leaderboard UI", () => {
  it("renders a validated cache immediately and retains it through a Graph outage", async () => {
    writeLeaderboardCache(localStorage, cachedSnapshot());
    const elements = mountElements();
    let resolveRequest: ((response: Response) => void) | undefined;
    const pending = vi.fn(
      () =>
        new Promise<Response>((resolve) => {
          resolveRequest = resolve;
        }),
    ) as typeof fetch;
    const controller = initializeLeaderboard(elements, {
      config: { ...config, graphEndpoint: "https://example.test/graphql" },
      fetch: pending,
      now: () => new Date("2026-08-11T12:01:00Z"),
    });
    expect(elements.ranked.textContent).toContain("Cache Tentacle");
    expect(elements.ranked.textContent).toContain("UWU1");
    expect(elements.ranked.textContent).toContain("0.00");
    expect(elements.status.textContent).toBe("REFRESHING");
    expect(pending).toHaveBeenCalledTimes(1);
    resolveRequest?.(new Response("unavailable", { status: 503 }));
    await controller.refresh();
    expect(elements.status.textContent).toBe("STALE");
    expect(elements.ranked.textContent).toContain("Cache Tentacle");
    controller.dispose();
  });

  it("groups shared identities and clearly separates suspended identities", () => {
    const snapshot = cachedSnapshot();
    const first = snapshot.rankedWallets[0].identities[0];
    snapshot.rankedWallets[0].identities.push({
      ...structuredClone(first),
      agentId: "2",
      tentacleId: "tentacle_second",
      profile: { ...first.profile, name: "Second identity" },
    });
    snapshot.suspended.push({
      ...structuredClone(first),
      agentId: "3",
      agentWallet: ZERO_ADDRESS,
      rawBalance: "0",
      tentacleId: "tentacle_suspended",
      profile: { ...first.profile, name: "Suspended identity" },
    });
    writeLeaderboardCache(localStorage, snapshot);
    const elements = mountElements();
    const controller = initializeLeaderboard(elements, {
      config,
      now: () => new Date("2026-08-11T12:01:00Z"),
    });
    expect(elements.ranked.textContent).toContain("SHARED WALLET ×2");
    expect(elements.ranked.textContent).toContain("Agent #1");
    expect(elements.ranked.textContent).toContain("Agent #2");
    expect(elements.suspended.textContent).toContain("WALLET UNVERIFIED");
    expect(elements.suspended.textContent).toContain("NONE");

    elements.verification.value = "suspended";
    elements.verification.dispatchEvent(new Event("input"));
    expect(elements.ranked.childElementCount).toBe(0);
    expect(elements.suspended.textContent).toContain("Suspended identity");
    elements.verification.value = "all";
    elements.funding.value = "unfunded";
    elements.funding.dispatchEvent(new Event("input"));
    expect(elements.suspended.childElementCount).toBe(0);
    controller.dispose();
  });

  it("atomically replaces the cache only after a complete background refresh", async () => {
    writeLeaderboardCache(localStorage, cachedSnapshot());
    const fixture = readFileSync(
      resolve(process.cwd(), "../subgraph/fixtures/leaderboard-v1.json"),
      "utf8",
    );
    const elements = mountElements();
    const controller = initializeLeaderboard(elements, {
      config: { ...config, graphEndpoint: "https://example.test/graphql" },
      fetch: vi.fn(async () => new Response(fixture)) as typeof fetch,
      now: () => new Date("2026-08-11T12:01:00Z"),
    });
    await controller.refresh();
    expect(elements.status.textContent).toBe("CURRENT");
    expect(elements.ranked.textContent).toContain("Fixture Tentacle");
    expect(elements.ranked.textContent).toContain("1 active · 0 revoked · 1 total");
    expect(elements.ranked.textContent).toContain("informational provenance only");
    expect(elements.ranked.textContent).toContain("Recent public event sample · 1 shown of 1");
    expect(readLeaderboardCache(localStorage)?.rankedWallets[0].representativeAgentId).toBe("7");
    controller.dispose();
  });

  it("surfaces subgraph indexing errors without overwriting a valid cache", async () => {
    writeLeaderboardCache(localStorage, cachedSnapshot());
    const fixture = JSON.parse(
      readFileSync(resolve(process.cwd(), "../subgraph/fixtures/leaderboard-v1.json"), "utf8"),
    ) as { data: { _meta: { hasIndexingErrors: boolean } } };
    fixture.data._meta.hasIndexingErrors = true;
    const elements = mountElements();
    const controller = initializeLeaderboard(elements, {
      config: { ...config, graphEndpoint: "https://example.test/graphql" },
      fetch: vi.fn(async () => new Response(JSON.stringify(fixture))) as typeof fetch,
    });
    await controller.refresh();
    expect(elements.status.textContent).toBe("INDEXING ERROR");
    expect(elements.ranked.textContent).toContain("Cache Tentacle");
    expect(readLeaderboardCache(localStorage)?.rankedWallets[0].representativeAgentId).toBe("1");
    controller.dispose();
  });

  it("renders hostile profile text as text and rejects hostile image schemes", () => {
    const snapshot = cachedSnapshot('<img src=x onerror="boom">');
    snapshot.rankedWallets[0].identities[0].profile.image =
      "https://trusted-gateway.example/ipfs/oversized-public-image";
    writeLeaderboardCache(localStorage, snapshot);
    const elements = mountElements();
    const controller = initializeLeaderboard(elements, {
      config,
      now: () => new Date("2026-08-11T12:01:00Z"),
    });
    expect(elements.ranked.textContent).toContain('<img src=x onerror="boom">');
    expect(elements.ranked.querySelector(".tentacle-heading img")).toBeNull();
    expect(elements.ranked.querySelector<HTMLImageElement>(".tentacle-avatar")?.src).toContain(
      "/icons/cthuwu-192.png",
    );
    expect(elements.ranked.innerHTML).not.toContain("trusted-gateway.example");
    controller.dispose();
  });

  it("filters by search, funded state, protocol, and shared-wallet state", () => {
    writeLeaderboardCache(localStorage, cachedSnapshot("Needle Tentacle"));
    const elements = mountElements();
    const controller = initializeLeaderboard(elements, { config });
    elements.search.value = "missing";
    elements.search.dispatchEvent(new Event("input"));
    expect(elements.ranked.childElementCount).toBe(0);
    expect(elements.empty.hidden).toBe(false);
    elements.search.value = "0x1111";
    elements.search.dispatchEvent(new Event("input"));
    expect(elements.ranked.childElementCount).toBe(1);
    elements.shared.value = "shared";
    elements.shared.dispatchEvent(new Event("input"));
    expect(elements.ranked.childElementCount).toBe(0);
    controller.dispose();
  });
});
