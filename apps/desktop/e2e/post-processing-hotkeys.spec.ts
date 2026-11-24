import { expect, test } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

// ADR 028：原「设置 → 其他热键」拆开了——语音输入的四个热键在「语音输入 → 输入」，
// 启动 / 停止本地语音识别的热键在「设置 → 模型 → 语音识别」（只在本地识别时出现；语音识别归本体，交接第十一节第 6 条），
// 停止热键在「设置 → 系统 → 停下」。
test("named hotkey rows render with defaults and can be configured/cleared", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page);
  const rows = page.getByTestId("voice-hotkeys");

  // 四个语音输入的热键行：频道切换（默认生效）/ 重发 / 文字处理开关 / 切换流程
  await expect(rows.getByText("频道切换热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("重发最近一条热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("文字处理开关热键", { exact: true })).toBeVisible();
  await expect(rows.getByText("切换文字处理流程热键", { exact: true })).toBeVisible();
  // 语音输入页没有启动热键（在「设置 → 模型 → 语音识别」，见最后）
  await expect(page.getByTestId("asr-toggle-hotkey")).toHaveCount(0);

  // 默认：重发、开关、切流程均为「未设置」，频道切换沿用默认 Shift+CapsLock
  const toggleInput = page.getByTestId("hotkey-input-post-process-toggle");
  const cycleInput = page.getByTestId("hotkey-input-post-process-cycle");
  await expect(page.getByTestId("hotkey-input-channel-cycle")).toHaveValue("Shift+CapsLock");
  await expect(page.getByTestId("hotkey-input-resend-last")).toHaveValue("");
  await expect(toggleInput).toHaveValue("");
  await expect(cycleInput).toHaveValue("");
  for (const id of ["resend-last", "post-process-toggle", "post-process-cycle"]) {
    await expect(page.getByTestId(`hotkey-status-${id}`), id).toHaveText("未设置");
  }

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

  // 与语音输入热键冲突：预检拒绝并 toast（主键默认 CapsLock）
  await toggleInput.fill("CapsLock");
  await toggleInput.press("Enter");
  await expect(
    page.getByText("文字处理开关热键与语音输入热键（CapsLock）冲突，请换一个", { exact: true }),
  ).toBeVisible();

  // 清除后回到未设置
  await page.getByTestId("hotkey-clear-post-process-toggle").click();
  await expect(page.getByText("已关闭文字处理开关热键", { exact: true })).toBeVisible();
  await expect(toggleInput).toHaveValue("");

  // 本地识别（默认）：启动热键在「设置 → 模型 → 语音识别」，默认未设置，没启动时也有效
  await openSettingsTab(page, "models");
  await expect(page.getByTestId("asr-toggle-hotkey")).toContainText("启动热键");
  await expect(page.getByTestId("hotkey-input-runtime-toggle")).toHaveValue("");
  await expect(page.getByTestId("hotkey-status-runtime-toggle")).toHaveText("未设置");
});
