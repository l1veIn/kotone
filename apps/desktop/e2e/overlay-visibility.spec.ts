import { expect, test } from "@playwright/test";
import { openVoiceInput } from "./nav";

test("overlay can be disabled completely", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page);

  const neverButton = page.getByRole("button", { name: /不显示.*热键和语音输入照常/ });
  await expect(neverButton).toBeVisible();
  await neverButton.click();

  await expect(page.getByText("输入条已切换：不显示", { exact: true })).toBeVisible();
  await expect(neverButton).toHaveAttribute("aria-pressed", "true");
});
