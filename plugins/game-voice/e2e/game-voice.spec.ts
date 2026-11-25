// 游戏语音插件（ADR 020、038 第 4 步）：角色页的「交互」（原「设置 → 她怎么回答」）里不再有游戏语音；左侧栏「插件」里有它的卡片（默认关），
// 从卡片点开它的页面（左侧栏不放插件）。打开插件 → 没装虚拟麦克风 → 自动安装 → 转发在运行 → 改 / 改回 Windows 默认麦克风 → 测试音。
// 设置写进 config.json `plugins.game-voice`（旧的 `voiceChat` 没有了）。浏览器预览里安装与设备是模拟的（ipc/gameVoiceMock.ts）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";

/** 截图放在仓库的 target/plugin-shots/step4/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step4", `${name}.png`);

/** 页面里的设置（字符串形式求值：这个目录的测试按 CommonJS 编译，函数里的 import() 会被改写）。 */
async function settings(page: Page): Promise<Record<string, unknown> & {
  plugins?: Record<string, { enabled?: boolean; settings?: Record<string, unknown> }>;
}> {
  return page.evaluate("import('/src/lib/ipc.ts').then((ipc) => ipc.getSettings())");
}

test("game voice is a plugin: off by default, turn it on, install the virtual mic, route the Windows default mic and test it", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // 角色页的「交互」（原「设置 → 她怎么回答」，交接第十一节第 8 条）：没有游戏语音了
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-reply").click();
  await expect(page.getByTestId("settings-reply")).toBeVisible();
  await expect(page.getByTestId("settings-reply")).not.toContainText("游戏语音");
  await expect(page.getByTestId("voice-chat-settings")).toHaveCount(0);

  // 左侧栏「插件」：游戏语音的卡片，默认关
  await page.getByTestId("settings-nav-plugins").click();
  const card = page.getByTestId("plugins-card-game-voice");
  await expect(card).toContainText("游戏语音");
  await expect(card.getByTestId("plugins-card-game-voice-off")).toBeVisible();
  await expect(card.getByTestId("plugins-card-game-voice-enabled")).toHaveAttribute("aria-checked", "false");
  await page.screenshot({ path: shot(testInfo, "settings-plugins-game-voice") });

  // 从卡片点开插件的 App：关着时只有开关和一句说明；左侧栏没有单个插件，亮的是「插件」
  await expect(page.getByTestId("app-sidebar").locator('[data-testid^="settings-nav-plugin:"]')).toHaveCount(0);
  await card.getByTestId("plugins-card-game-voice-open").click();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
  const pluginPage = page.getByTestId("plugin-page-game-voice");
  await expect(pluginPage.getByRole("heading", { name: "游戏语音", exact: true })).toBeVisible();
  await expect(pluginPage.getByTestId("plugin-game-voice-off")).toBeVisible();
  await expect(pluginPage.getByTestId("voice-chat-settings")).toHaveCount(0);

  // 打开插件：还没装虚拟麦克风
  await pluginPage.getByTestId("plugin-game-voice-enabled").click();
  await expect(pluginPage.getByTestId("plugin-game-voice-enabled")).toHaveAttribute("aria-checked", "true");
  const section = pluginPage.getByTestId("voice-chat-settings");
  await expect(section).toBeVisible();
  await expect(pluginPage.getByTestId("voice-chat-state")).toContainText("还没有安装");
  await expect(section).toContainText("VB-Audio 的捐赠软件");

  await pluginPage.getByTestId("voice-chat-install").click();
  await expect(pluginPage.getByTestId("voice-chat-state")).toContainText("运行中");

  await pluginPage.getByTestId("voice-chat-use-cable").click();
  await expect(pluginPage.getByTestId("voice-chat-restore-mic")).toBeVisible();
  await pluginPage.getByTestId("voice-chat-test").click();
  await expect(page.getByText("通了")).toBeVisible();
  await page.screenshot({ path: shot(testInfo, "game-voice-page") });
  await pluginPage.getByTestId("voice-chat-restore-mic").click();
  await expect(pluginPage.getByTestId("voice-chat-use-cable")).toBeVisible();

  // 只混她的声音：写进插件自己的设置
  await pluginPage.getByTestId("voice-chat-include-mic").click();
  await expect(pluginPage.getByTestId("voice-chat-mic")).toHaveCount(0);
  await expect.poll(async () => (await settings(page)).plugins?.["game-voice"]).toMatchObject({
    enabled: true,
    settings: { includeMic: false },
  });
  expect((await settings(page)).voiceChat).toBeUndefined();

  // 关掉插件：又只剩开关
  await pluginPage.getByTestId("plugin-game-voice-enabled").click();
  await expect(pluginPage.getByTestId("plugin-game-voice-off")).toBeVisible();
  await expect(pluginPage.getByTestId("voice-chat-settings")).toHaveCount(0);
  expect((await settings(page)).plugins?.["game-voice"]?.enabled).toBe(false);
});

test("old 游戏语音 locations and her ui.open land on the game voice plugin page", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  // 旧对话里的“去看看：设置 → 她怎么回答 → 游戏语音”
  await page.evaluate("import('/src/lib/stores/navigation.ts').then((n) => n.openLocation('设置 → 她怎么回答 → 游戏语音'))");
  await expect(page.getByTestId("plugin-page-game-voice")).toBeVisible();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
  // 她打开插件的页面：玩家正在插件页，不切过去（共享资源评估第二节第 6 条）；点提示上的“打开”在聊天页的右侧栏打开
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/game-voice/home' })")).toBe("waiting");
  await expect(page.getByTestId("plugin-page-game-voice")).toBeVisible();
  await page.locator("[data-sonner-toast]").filter({ hasText: "游戏语音" }).getByRole("button", { name: "打开" }).click();
  await expect(page.getByTestId("plugin-view-game-voice").getByTestId("plugin-game-voice-enabled")).toBeVisible();
});
