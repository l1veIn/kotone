import { expect, test } from "@playwright/test";

test("overlay can be disabled completely", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  const neverButton = page.getByRole("button", { name: /不显示.*完全隐藏悬浮窗/ });
  await expect(neverButton).toBeVisible();
  await neverButton.click();

  await expect(page.getByText("悬浮窗已切换：不显示", { exact: true })).toBeVisible();
  await expect(neverButton).toHaveClass(/ring-kotone-cyan\/60/);
});
