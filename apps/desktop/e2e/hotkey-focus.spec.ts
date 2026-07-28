import { expect, test } from "@playwright/test";

test("a configured Tab hotkey does not move focus inside Kotone", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  const input = page.locator("#hotkey-input");
  await input.fill("Tab");
  const save = page.getByRole("button", { name: "保存并生效", exact: true });
  await save.click();
  await expect(page.getByText("当前：Tab", { exact: true })).toBeVisible();

  await save.focus();
  await expect(save).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(save).toBeFocused();
});
