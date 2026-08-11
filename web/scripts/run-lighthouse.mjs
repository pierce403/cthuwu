import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { chromium } from "@playwright/test";

const require = createRequire(import.meta.url);
const cli = require.resolve("lighthouse/cli/index.js");
const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("run-lighthouse must run through npm");

const url = "http://127.0.0.1:4173/";
const outputDirectory = "test-results/lighthouse";
const outputPath = `${outputDirectory}/report.json`;
const chromePath = chromium.executablePath();
if (!existsSync(chromePath)) {
  throw new Error("Pinned Chromium is missing; run `npx playwright install chromium`");
}
mkdirSync(outputDirectory, { recursive: true });
rmSync(outputPath, { force: true });

const server = spawn(
  process.execPath,
  [
    npmCli,
    "run",
    "preview",
    "--",
    "--host",
    "127.0.0.1",
    "--port",
    "4173",
    "--strictPort",
  ],
  {
    cwd: process.cwd(),
    detached: process.platform !== "win32",
    stdio: "ignore",
  },
);

try {
  await waitForServer(url, server);
  const result = spawnSync(
    process.execPath,
    [
      cli,
      url,
      "--chrome-flags=--headless=new --no-sandbox --disable-dev-shm-usage",
      "--only-categories=accessibility,best-practices",
      "--output=json",
      `--output-path=${outputPath}`,
      "--quiet",
    ],
    {
      cwd: process.cwd(),
      env: { ...process.env, CHROME_PATH: chromePath },
      stdio: "inherit",
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Lighthouse exited with status ${result.status ?? 1}`);

  const report = JSON.parse(readFileSync(outputPath, "utf8"));
  assertScore(report, "accessibility", 0.95);
  assertScore(report, "best-practices", 0.9);
} finally {
  if (server.pid) {
    try {
      if (process.platform === "win32") server.kill("SIGTERM");
      else process.kill(-server.pid, "SIGTERM");
    } catch {
      // The preview process may already have exited after a failed audit.
    }
  }
}

async function waitForServer(target, processHandle) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (processHandle.exitCode !== null) throw new Error("static preview exited before Lighthouse");
    try {
      const response = await fetch(target);
      if (response.ok) return;
    } catch {
      // The server is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("static preview did not become ready for Lighthouse");
}

function assertScore(report, category, minimum) {
  const score = report?.categories?.[category]?.score;
  if (typeof score !== "number") throw new Error(`Lighthouse omitted ${category}`);
  console.log(`Lighthouse ${category}: ${score.toFixed(2)} (required ${minimum.toFixed(2)})`);
  if (score < minimum) {
    throw new Error(`Lighthouse ${category} score ${score.toFixed(2)} is below ${minimum}`);
  }
}
