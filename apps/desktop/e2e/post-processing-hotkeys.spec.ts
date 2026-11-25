import { expect, test } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

// ADR 040 D3：热键只在「设置 → 热键」改。语音输入的四个热键在语音输入页的「其他热键」只显示现在的键和“去改”，
// 启动 / 停止本地语音识别的热键在「设置 → 模型 → 语音识别」也只显示（只在本地识别时出现）。
test("named hotkey rows render with defaults and can be configured/cleared", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page);
  const rows = page.getByTestId("voice-hotkeys");

  // 四个语音输入的热键行：频道切换（老用户迁移来的 Shift+CapsLock）/ 重发 / 文字处理开关 / 切换流程
  await expect(rows.getByText("频道切换热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("重发最近一条热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("文字处理开关热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("切换文字处理流程热键", { exact: true })).toBeVisible();
  await expect(rows.getByTestId("hotkey-link-key-voice-input.channel-cycle")).toHaveText("Shift + CapsLock");
  await expect(rows.getByTestId("hotkey-link-key-voice-input.resend-last")).toHaveText("未设置");
  await expect(rows.locator("input")).toHaveCount(0);
  // 语音输入页没有启动热键（在「设置 → 模型 → 语音识别」，见最后）
  await expect(page.getByTestId("asr-toggle-hotkey")).toHaveCount(0);

  // “去改”到「设置 → 热键」那一行
  await rows.getByTestId("hotkey-link-goto-voice-input.post-process-toggle").click();
  await expect(page.getByTestId("advanced-nav-hotkeys")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("hotkeys-row-voice-input.post-process-toggle")).toHaveAttribute("data-focused", "true");

  // 默认：重发、开关、切流程均为「未设置」
  const toggleInput = page.getByTestId("hotkey-input-voice-input.post-process-toggle");
  const cycleInput = page.getByTestId("hotkey-input-voice-input.post-process-cycle");
  await expect(page.getByTestId("hotkey-input-voice-input.channel-cycle")).toHaveValue("Shift+CapsLock");
  await expect(page.getByTestId("hotkey-input-voice-input.resend-last")).toHaveValue("");
  await expect(toggleInput).toHaveValue("");
  await expect(cycleInput).toHaveValue("");
  for (const id of ["voice-input.resend-last", "voice-input.post-process-toggle", "voice-input.post-process-cycle"]) {
    await expect(page.getByTestId(`hotkey-status-${id}`), id).toHaveText("未设置");
  }

  // 录入文字处理开关热键
  await toggleInput.fill("F9");
  await toggleInput.press("Enter");
  await expect(page.getByText("文字处理开关热键已保存并生效：F9", { exact: true })).toBeVisible();
  await expect(page.getByTestId("hotkey-status-voice-input.post-process-toggle")).toHaveText("当前生效：F9");

  // 录入切换流程热键
  await cycleInput.fill("F10");
  await cycleInput.press("Enter");
  await expect(page.getByText("切换文字处理流程热键已保存并生效：F10", { exact: true })).toBeVisible();
  await expect(page.getByTestId("hotkey-status-voice-input.post-process-cycle")).toHaveText("当前生效：F10");

  // 与语音输入热键冲突：后端查出来，不存并说和谁撞了
  await toggleInput.fill("CapsLock");
  await toggleInput.press("Enter");
  await expect(
    page.getByText("文字处理开关热键与语音输入热键（CapsLock）冲突，请换一个", { exact: true }),
  ).toBeVisible();

  // 清除后回到未设置
  await page.getByTestId("hotkey-clear-voice-input.post-process-toggle").click();
  await expect(page.getByText("已关闭文字处理开关热键", { exact: true })).toBeVisible();
  await expect(toggleInput).toHaveValue("");

  // 语音输入页跟着变
  await openVoiceInput(page);
  await expect(page.getByTestId("hotkey-link-key-voice-input.post-process-cycle")).toHaveText("F10");

  // 本地识别（默认）：启动热键在「设置 → 模型 → 语音识别」只显示，默认未设置
  await openSettingsTab(page, "models");
  await expect(page.getByTestId("asr-toggle-hotkey")).toContainText("启动热键");
  await expect(page.getByTestId("hotkey-link-key-kotone.speech-toggle")).toHaveText("未设置");
});
