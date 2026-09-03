import { expect, test } from "@playwright/test";

test("sound feedback cards render with defaults and per-item toggles", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  // 高级 → 音效 tab
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-sound").click();

  // 两张卡片：录制音 / 发送音，默认均开启
  const recordToggle = page.getByRole("checkbox", { name: "录制音" });
  const sendToggle = page.getByRole("checkbox", { name: "发送音" });
  await expect(recordToggle).toBeVisible();
  await expect(recordToggle).toBeChecked();
  await expect(sendToggle).toBeChecked();

  // 默认下拉：上扬（录制）/ 下沉（发送）
  const recordSelect = page.getByRole("button", { name: "录制音音效" });
  const sendSelect = page.getByRole("button", { name: "发送音音效" });
  await expect(recordSelect).toHaveText("上扬");
  await expect(sendSelect).toHaveText("下沉");

  // 下拉切换「啁啾」→ 仅保存（试听走播放键）
  await recordSelect.click();
  await page.getByRole("option", { name: "啁啾" }).click();
  await expect(page.getByText("录制音已切换", { exact: true })).toBeVisible();
  await expect(recordSelect).toHaveText("啁啾");

  // 播放键试听（当前选中音效）
  await page.getByRole("button", { name: "试听录制音" }).click();
  await page.getByRole("button", { name: "试听发送音" }).click();

  // 截图：默认双开 + 刚切换的音效（供维护者确认 UI）
  await page.screenshot({
    path: "test-results/sound-feedback-tab.png",
    clip: { x: 180, y: 0, width: 720, height: 700 },
  });

  // 独立关闭发送音（录制音保持开启）
  await sendToggle.uncheck({ force: true });
  await expect(page.getByText("发送音已关闭", { exact: true })).toBeVisible();
  await expect(sendToggle).not.toBeChecked();
  await expect(recordToggle).toBeChecked();
});
