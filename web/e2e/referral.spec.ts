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

for (const viewport of [{ width: 1876, height: 1344 }, { width: 1536, height: 1100 }]) {
  test(`aligns the invitation and mascot at ${viewport.width}×${viewport.height}`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport);
    await page.route(/^https:\/\//u, (route) => route.abort("blockedbyclient"));
    await page.goto("/#r=0x2222222222222222222222222222222222222222", { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("region", { name: "Your invitation" })).toBeVisible();
    const buddy = (await page.locator(".buddy").boundingBox())!;
    const chat = (await page.locator("#chat").boundingBox())!;
    expect(Math.abs(chat.y - buddy.y)).toBeLessThan(2);
    const bottom = Math.max(chat.y + chat.height, buddy.y + buddy.height);
    expect(Math.abs(chat.y - (viewport.height - bottom))).toBeLessThan(3);
    await page.screenshot({ path: testInfo.outputPath("referral-large.png"), fullPage: true });
  });
}
