import { expect, test } from "@playwright/test";

test("referral welcome keeps chat and composer in the first viewport", async ({ page }, testInfo) => {
  await page.route(/^https:\/\//u, (route) => route.abort("blockedbyclient"));
  await page.goto("/#r=0x2222222222222222222222222222222222222222", { waitUntil: "domcontentloaded" });
  await expect(page.getByRole("region", { name: "Your invitation" })).toBeVisible();
  const chat = await page.locator("#chat").boundingBox();
  const composer = await page.locator("#message").boundingBox();
  const viewport = page.viewportSize()!;
  expect(chat).not.toBeNull();
  expect(composer).not.toBeNull();
  expect(chat!.y).toBeLessThan(viewport.height / 3);
  expect(composer!.y + composer!.height).toBeLessThan(viewport.height);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("referral.png"), fullPage: true });
});
