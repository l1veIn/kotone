import { expect, test } from "@playwright/test";

// 琴音陪玩 → 设置 → 语音唤醒（ADR 017）：开关、模型下载、唤醒词；设置写进 assistant.wake。
test("voice wake can be enabled, its model downloaded and the phrase changed", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-talk").click();
  await expect(page.locator("#wake-settings")).toContainText("语音唤醒");
  await expect(page.getByTestId("wake-status")).toContainText("未开启");

  await page.getByTestId("wake-enabled").click();
  await expect(page.getByTestId("wake-status")).toContainText("还没有下载唤醒模型");
  await page.getByTestId("wake-download").click();
  await expect(page.getByTestId("wake-status")).toContainText("等待语音识别启动");
  await expect(page.getByTestId("wake-download")).toHaveCount(0);

  await page.getByLabel("唤醒词").click();
  await page.getByRole("option", { name: "你好琴音" }).click();
  await expect(page.getByTestId("talk-page")).toContainText("说“你好琴音”叫醒琴音");
  await page.getByTestId("wake-enabled").scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("wake-settings.png") });

  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.assistant.wake).toMatchObject({ enabled: true, phrase: "nihao-qinyin", sensitivity: "normal", ack: "voice" });
});

test("progress speech can be turned off", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  const toggle = page.getByTestId("agent-progress-speech");
  await expect(toggle).toHaveAttribute("data-state", "checked");
  await toggle.click();
  await expect(toggle).toHaveAttribute("data-state", "unchecked");
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.agent.progressSpeech).toBe(false);
});
