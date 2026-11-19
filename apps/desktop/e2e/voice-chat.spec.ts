import { expect, test } from "@playwright/test";
import { openSettingsTab } from "./nav";

// 设置 → 她怎么回答 → 游戏语音（ADR 020 / 028）：自动安装 VB-CABLE、开关、改 / 改回 Windows 默认麦克风、测试。
// 浏览器预览里安装与设备是模拟的；设置写进 config.json `voiceChat`。
test("game voice chat can be installed, enabled, routed to the Windows default mic and tested", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "reply");
  const section = page.getByTestId("voice-chat-settings");
  await expect(page.locator("#voice-chat-settings")).toContainText("游戏语音");
  await expect(page.getByTestId("voice-chat-state")).toContainText("还没有安装");
  await expect(section).toContainText("VB-Audio 的捐赠软件");
  await expect(page.getByTestId("voice-chat-enabled")).toBeDisabled();

  await page.getByTestId("voice-chat-install").click();
  await expect(page.getByTestId("voice-chat-state")).toContainText("未开启");
  await page.getByTestId("voice-chat-enabled").click();
  await expect(page.getByTestId("voice-chat-state")).toContainText("运行中");

  await page.getByTestId("voice-chat-use-cable").click();
  await expect(page.getByTestId("voice-chat-restore-mic")).toBeVisible();
  await page.getByTestId("voice-chat-test").click();
  await expect(page.getByText("通了")).toBeVisible();
  await page.getByTestId("voice-chat-restore-mic").click();
  await expect(page.getByTestId("voice-chat-use-cable")).toBeVisible();

  await section.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("voice-chat-settings.png") });
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.voiceChat).toMatchObject({ enabled: true, includeMic: true, micDeviceId: "default", micGain: 100 });
});
