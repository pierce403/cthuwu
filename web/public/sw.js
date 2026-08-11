const CACHE_NAME = "cthuwu-shell-v2";
const OFFLINE_PAGE = "/offline.html";
const OFFLINE_ASSETS = [
  OFFLINE_PAGE,
  "/offline-leaderboard.js",
  "/manifest.webmanifest",
  "/icons/cthuwu-192.png",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(OFFLINE_ASSETS)),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(
          names
            .filter(
              (name) =>
                (name.startsWith("cthuwu-offline-") || name.startsWith("cthuwu-shell-")) &&
                name !== CACHE_NAME,
            )
            .map((name) => caches.delete(name)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") self.skipWaiting();
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request).catch(async () => (await caches.match(OFFLINE_PAGE)) ?? Response.error()),
    );
    return;
  }

  if (OFFLINE_ASSETS.includes(url.pathname)) {
    // These exact same-origin paths are an explicit shell allowlist. Ignore the
    // host's Vary: Origin response header so a precached module request remains
    // available when the browser supplies an Origin header while offline.
    event.respondWith(
      caches.match(url.pathname, { ignoreVary: true }).then((cached) => cached || fetch(request)),
    );
  }
});
