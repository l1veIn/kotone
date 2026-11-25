import { expect, test } from "@playwright/test";
import { openSettingsTab } from "./nav";

test("a configured Tab hotkey does not install a browser-local listener", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 语音输入热键在「设置 → 热键」改（ADR 040 D3）
  await openSettingsTab(page, "hotkeys");

  const input = page.getByTestId("hotkey-input-voice-input.talk");
  await input.fill("Tab");
  await input.press("Enter");
  await expect(page.getByTestId("hotkey-status-voice-input.talk")).toContainText("Tab");

  const clear = page.getByTestId("hotkey-clear-voice-input.talk");
  await clear.focus();
  await expect(clear).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(clear).not.toBeFocused();
});
