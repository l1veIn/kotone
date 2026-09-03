import { expect, test } from "@playwright/test";

test("autostart toggle reflects state and toggles", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  // 高级 → 系统（默认）
  await page.getByTestId("settings-nav-advanced").click();

  const toggle = page.getByLabel("开机自动启动");
  await expect(toggle).toBeVisible();
  // 默认关闭
  await expect(toggle).not.toBeChecked();

  // 开启：开关 + 回读真值保持开启
  await toggle.check({ force: true });
  await expect(page.getByText("已开启开机自动启动", { exact: true })).toBeVisible();
  await expect(toggle).toBeChecked();

  // 截图：开启态（供维护者确认 UI）
  await page.screenshot({
    path: "test-results/autostart-on.png",
    clip: { x: 180, y: 0, width: 720, height: 700 },
  });

  // 关闭
  await toggle.uncheck({ force: true });
  await expect(page.getByText("已关闭开机自动启动", { exact: true })).toBeVisible();
  await expect(toggle).not.toBeChecked();
});
