import { expect, test } from "@playwright/test";

test("named hotkey rows render with defaults and can be configured/cleared", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-hotkeys").click();

  // 四个具名热键行都在：频道切换（默认生效）/ 重发 / 文字处理开关 / 切换流程
  await expect(page.getByText("频道切换热键", { exact: true })).toBeVisible();
  await expect(page.getByText("重发最近一条热键", { exact: true })).toBeVisible();
  await expect(page.getByText("文字处理开关热键", { exact: true })).toBeVisible();
  await expect(page.getByText("切换文字处理流程热键", { exact: true })).toBeVisible();

  // 默认：开关与切流程均为「未设置」，频道切换沿用默认 Shift+CapsLock
  const toggleInput = page.getByTestId("hotkey-input-post-process-toggle");
  const cycleInput = page.getByTestId("hotkey-input-post-process-cycle");
  await expect(toggleInput).toHaveValue("");
  await expect(cycleInput).toHaveValue("");
  await expect(
    page.getByText("未设置（默认关闭，不会误触发）", { exact: true }),
  ).toHaveCount(3);

  // 录入文字处理开关热键
  await toggleInput.fill("F9");
  await toggleInput.press("Enter");
  await expect(page.getByText("文字处理开关热键已保存并生效：F9", { exact: true })).toBeVisible();
  await expect(page.getByText("当前生效：F9", { exact: true })).toBeVisible();

  // 录入切换流程热键
  await cycleInput.fill("F10");
  await cycleInput.press("Enter");
  await expect(page.getByText("切换文字处理流程热键已保存并生效：F10", { exact: true })).toBeVisible();
  await expect(page.getByText("当前生效：F10", { exact: true })).toBeVisible();

  // 与录制热键冲突：预检拒绝并 toast（主键默认 CapsLock）
  await toggleInput.fill("CapsLock");
  await toggleInput.press("Enter");
  await expect(
    page.getByText("文字处理开关热键与录制热键（CapsLock）冲突，请换一个", { exact: true }),
  ).toBeVisible();

  // 清除后回到未设置
  await page.getByTestId("hotkey-clear-post-process-toggle").click();
  await expect(page.getByText("已关闭文字处理开关热键", { exact: true })).toBeVisible();
  await expect(toggleInput).toHaveValue("");
});