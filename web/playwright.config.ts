import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["line"]] : "list",
  outputDir: "test-results/playwright",
  use: {
    baseURL: "http://127.0.0.1:4173",
    ...(process.env.CTHUWU_TEST_CHROMIUM ? { launchOptions: { executablePath: process.env.CTHUWU_TEST_CHROMIUM, args: ["--no-sandbox", "--disable-dev-shm-usage"] } } : {}),
    serviceWorkers: "allow",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "mobile-chromium", use: { ...devices["Pixel 7"], browserName: "chromium" } },
    { name: "desktop-chromium", use: { ...devices["Desktop Chrome"], browserName: "chromium" } },
  ],
  webServer: {
    command: "npm run preview -- --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173/",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
