import { expect, test } from "@playwright/test";

test("the check-for-updates button is always visible and reports the latest version", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByRole("button", { name: "关于", exact: true }).click();

  const checkUpdates = page.getByTestId("check-updates");
  await expect(checkUpdates).toBeVisible();
  await expect(checkUpdates).toBeEnabled();

  await checkUpdates.click();
  await expect(page.getByText("✓ 已是最新版本（v0.1.3）", { exact: true })).toBeVisible();
  await expect(checkUpdates).toBeVisible();
  await expect(checkUpdates).toBeEnabled();
});
