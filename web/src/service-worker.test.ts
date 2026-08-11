import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

type WorkerHandler = (event: Record<string, unknown>) => void;

function loadWorker(fetcher: typeof fetch = vi.fn() as typeof fetch) {
  const handlers = new Map<string, WorkerHandler>();
  const addAll = vi.fn(async () => undefined);
  const removeCache = vi.fn(async () => true);
  const offline = new Response("offline", { status: 200 });
  const match = vi.fn(async (request: Request | string) =>
    request === "/offline.html" || (typeof request !== "string" && request.url.endsWith("/offline.html"))
      ? offline
      : undefined,
  );
  const claim = vi.fn(async () => undefined);
  const skipWaiting = vi.fn();
  const self = {
    location: { origin: "https://cthuwu.app" },
    clients: { claim },
    skipWaiting,
    addEventListener: (type: string, handler: WorkerHandler) => handlers.set(type, handler),
  };
  const caches = {
    open: vi.fn(async () => ({ addAll })),
    keys: vi.fn(async () => ["cthuwu-shell-v1", "cthuwu-shell-v2", "unrelated"]),
    delete: removeCache,
    match,
  };
  runInNewContext(readFileSync(resolve(process.cwd(), "public/sw.js"), "utf8"), {
    self,
    caches,
    fetch: fetcher,
    URL,
    Response,
  });
  return { handlers, addAll, removeCache, match, claim, skipWaiting, offline };
}

describe("service worker privacy and offline behavior", () => {
  it("precaches only the bounded public shell and cleans obsolete Cthuwu caches", async () => {
    const worker = loadWorker();
    let install: Promise<unknown> | undefined;
    worker.handlers.get("install")?.({ waitUntil: (promise: Promise<unknown>) => (install = promise) });
    await install;
    expect(worker.addAll).toHaveBeenCalledWith([
      "/offline.html",
      "/offline-leaderboard.js",
      "/manifest.webmanifest",
      "/icons/cthuwu-192.png",
    ]);

    let activate: Promise<unknown> | undefined;
    worker.handlers.get("activate")?.({ waitUntil: (promise: Promise<unknown>) => (activate = promise) });
    await activate;
    expect(worker.removeCache).toHaveBeenCalledTimes(1);
    expect(worker.removeCache).toHaveBeenCalledWith("cthuwu-shell-v1");
    expect(worker.claim).toHaveBeenCalledTimes(1);
  });

  it("uses the offline page for failed navigation without intercepting Graph, XMTP, or arbitrary assets", async () => {
    const fetcher = vi.fn(async () => {
      throw new TypeError("offline");
    }) as typeof fetch;
    const worker = loadWorker(fetcher);
    const onFetch = worker.handlers.get("fetch")!;
    let response: Promise<Response> | undefined;
    onFetch({
      request: { method: "GET", mode: "navigate", url: "https://cthuwu.app/tentacles" },
      respondWith: (promise: Promise<Response>) => (response = promise),
    });
    await expect(response).resolves.toBe(worker.offline);
    expect(worker.match).toHaveBeenCalledWith("/offline.html");

    for (const request of [
      { method: "POST", mode: "cors", url: "https://gateway.thegraph.com/graphql" },
      { method: "GET", mode: "cors", url: "https://api.xmtp.network/messages" },
      { method: "GET", mode: "no-cors", url: "https://cthuwu.app/assets/private.bin" },
    ]) {
      const respondWith = vi.fn();
      onFetch({ request, respondWith });
      expect(respondWith).not.toHaveBeenCalled();
    }
  });

  it("activates a waiting update only after the controlled message", () => {
    const worker = loadWorker();
    worker.handlers.get("message")?.({ data: { type: "OTHER" } });
    expect(worker.skipWaiting).not.toHaveBeenCalled();
    worker.handlers.get("message")?.({ data: { type: "SKIP_WAITING" } });
    expect(worker.skipWaiting).toHaveBeenCalledTimes(1);
  });
});
