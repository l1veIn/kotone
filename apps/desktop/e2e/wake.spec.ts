import { expect, test } from "@playwright/test";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 设置 → 怎么叫她 → 语音唤醒（ADR 017 / 028）：开关、模型下载、唤醒词；设置写进 assistant.wake。
test("voice wake can be enabled, its model downloaded and the phrase changed", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "call");
  await expect(page.locator("#wake-settings")).toContainText("语音唤醒");
  await expect(page.getByTestId("wake-status")).toContainText("未开启");

  await page.getByTestId("wake-enabled").click();
  await expect(page.getByTestId("wake-status")).toContainText("还没有下载唤醒模型");
  await page.getByTestId("wake-download").click();
  // 本地识别还没启动（ADR 028）：不说“点右上角「启动」”，只说启动后开始听
  await expect(page.getByTestId("wake-status")).toContainText("本地语音识别启动后开始听");
  await expect(page.getByTestId("wake-status")).not.toContainText("右上角");
  await expect(page.getByTestId("wake-download")).toHaveCount(0);

  await page.getByLabel("唤醒词").click();
  await page.getByRole("option", { name: "你好琴音" }).click();
  await expect(page.getByTestId("settings-call")).toContainText("说“你好琴音”叫醒琴音，接着说要做的事。");
  await page.getByTestId("wake-enabled").scrollIntoViewIfNeeded();
  await screenshot(page, { path: testInfo.outputPath("wake-settings.png") });

  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.assistant.wake).toMatchObject({ enabled: true, phrase: "nihao-qinyin", sensitivity: "normal", ack: "voice" });
});

test("progress speech can be turned off", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 进度播报在 Agent 设置里（设置 → 权限）
  await openSettingsTab(page, "permissions");
  const toggle = page.getByTestId("agent-progress-speech");
  await expect(toggle).toHaveAttribute("data-state", "checked");
  await toggle.click();
  await expect(toggle).toHaveAttribute("data-state", "unchecked");
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.agent.progressSpeech).toBe(false);
});
