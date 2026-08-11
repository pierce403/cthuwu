import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { cachedSnapshot } from "../src/leaderboard-test-data";
import { LEADERBOARD_CACHE_KEY } from "../src/leaderboard-types";

const GRAPH_ENDPOINT = "https://graph.fixture.invalid/graphql";
const graphFixture = readFileSync(
  resolve(process.cwd(), "../subgraph/fixtures/leaderboard-v1.json"),
  "utf8",
);

test("renders a validated cache first, then refreshes the complete mobile leaderboard", async ({
  page,
}) => {
  const pageErrors: Error[] = [];
  const thirdPartyImages: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error));
  page.on("request", (request) => {
    if (request.resourceType() === "image" && !request.url().startsWith("http://127.0.0.1:4173")) {
      thirdPartyImages.push(request.url());
    }
  });
  await page.addInitScript(
    ({ key, value }) => localStorage.setItem(key, value),
    { key: LEADERBOARD_CACHE_KEY, value: JSON.stringify(cachedSnapshot("Cached First")) },
  );

  let releaseGraph: (() => void) | undefined;
  const graphGate = new Promise<void>((resolveGate) => {
    releaseGraph = resolveGate;
  });
  let graphRequested: (() => void) | undefined;
  const sawGraph = new Promise<void>((resolveRequest) => {
    graphRequested = resolveRequest;
  });
  await page.route(/^https:\/\//u, async (route) => {
    if (route.request().url() === GRAPH_ENDPOINT) {
      graphRequested?.();
      await graphGate;
      await route.fulfill({ status: 200, contentType: "application/json", body: graphFixture });
      return;
    }
    await route.abort("blockedbyclient");
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await sawGraph;
  await expect(page.getByText("Cached First", { exact: true }).first()).toBeVisible();
  await expect(page.locator("#leaderboard-state")).toHaveText("REFRESHING");

  await expect(page.getByLabel("Search", { exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Funding", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Status", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Protocol", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Wallet", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Sort", exact: true })).toBeVisible();
  await page.getByLabel("Search", { exact: true }).focus();
  await expect(page.getByLabel("Search", { exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("combobox", { name: "Funding", exact: true })).toBeFocused();

  releaseGraph?.();
  await expect(page.locator("#leaderboard-state")).toHaveText("CURRENT");
  await expect(page.getByText("Fixture Tentacle", { exact: true }).first()).toBeVisible();
  await expect(page.locator("#leaderboard-source")).toContainText("block 49768180");
  await expect(page.locator(".tentacle-reputation summary")).toContainText(
    "1 active · 0 revoked · 1 total",
  );
  await page.locator(".tentacle-reputation summary").click();
  await expect(page.getByText(/informational provenance only/u)).toBeVisible();
  await expect(page.getByText(/Recent public event sample · 1 shown of 1/u)).toBeVisible();

  await page.getByLabel("Search", { exact: true }).fill("missing identity");
  await expect(page.locator("#leaderboard-list")).toBeEmpty();
  await page.getByLabel("Search", { exact: true }).fill("7");
  await expect(page.getByText("Fixture Tentacle", { exact: true }).first()).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(
    true,
  );
  expect(thirdPartyImages).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test("serves an installable manifest, narrowly scoped worker, and cached offline shell", async ({
  context,
  page,
  request,
  baseURL,
}) => {
  if (!baseURL) throw new Error("Playwright baseURL is required");
  await page.route(/^https:\/\//u, async (route) => {
    if (route.request().url() === GRAPH_ENDPOINT) {
      await route.fulfill({ status: 200, contentType: "application/json", body: graphFixture });
      return;
    }
    await route.abort("blockedbyclient");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.locator("#leaderboard-state")).toHaveText("CURRENT");

  const manifestHref = await page.locator('link[rel="manifest"]').getAttribute("href");
  expect(manifestHref).toBe("/manifest.webmanifest");
  const manifestResponse = await request.get(`${baseURL}${manifestHref}`);
  expect(manifestResponse.ok()).toBe(true);
  expect(manifestResponse.headers()["content-type"]).toContain("application/manifest+json");
  const manifest = (await manifestResponse.json()) as {
    id: string;
    start_url: string;
    scope: string;
    display: string;
    icons: Array<{ src: string; sizes: string; purpose: string }>;
  };
  expect(manifest).toMatchObject({ id: "/", start_url: "/", scope: "/", display: "standalone" });
  expect(manifest.icons).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ sizes: "192x192", purpose: "any" }),
      expect.objectContaining({ sizes: "512x512", purpose: "any" }),
      expect.objectContaining({ sizes: "512x512", purpose: "maskable" }),
    ]),
  );
  for (const icon of manifest.icons) {
    const response = await request.get(`${baseURL}${icon.src}`);
    expect(response.ok()).toBe(true);
    expect(response.headers()["content-type"]).toBe("image/png");
    const bytes = await response.body();
    const [width, height] = icon.sizes.split("x").map(Number);
    expect(bytes.readUInt32BE(16)).toBe(width);
    expect(bytes.readUInt32BE(20)).toBe(height);
  }
  const workerResponse = await request.get(`${baseURL}/sw.js`);
  expect(workerResponse.ok()).toBe(true);
  expect(workerResponse.headers()["content-type"]).toContain("javascript");
  const offlineResponse = await request.get(`${baseURL}/offline.html`);
  expect(offlineResponse.ok()).toBe(true);
  expect(offlineResponse.headers()["content-type"]).toContain("text/html");

  expect(await page.evaluate(() => window.isSecureContext)).toBe(true);
  const registration = await page.evaluate(async () => {
    const ready = await navigator.serviceWorker.ready;
    return { scope: ready.scope, active: ready.active?.scriptURL };
  });
  expect(registration.scope).toBe(`${baseURL}/`);
  expect(registration.active).toBe(`${baseURL}/sw.js`);
  await expect
    .poll(() => page.evaluate(() => navigator.serviceWorker.controller?.scriptURL))
    .toBe(`${baseURL}/sw.js`);

  const cachedUrls = await page.evaluate(async () => {
    const urls: string[] = [];
    for (const name of await caches.keys()) {
      for (const response of await (await caches.open(name)).keys()) urls.push(response.url);
    }
    return urls;
  });
  expect(cachedUrls.some((url) => url.includes("graphql") || url.includes("xmtp"))).toBe(false);
  expect(cachedUrls.some((url) => url.endsWith("/offline.html"))).toBe(true);
  expect(cachedUrls.some((url) => url.endsWith("/offline-leaderboard.js"))).toBe(true);

  await context.setOffline(true);
  try {
    await page.goto("/offline-browser-check", { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("heading", { name: "The portal is offline" })).toBeVisible();
    await expect(page.getByText("Fixture Tentacle", { exact: true })).toBeVisible();
    await expect(page.locator("#offline-source")).toContainText("Base 8453 · block 49768180");
  } finally {
    await context.setOffline(false);
  }
});
