// 英雄联盟插件的 App（ADR 038 第 1 步）：从左侧栏「插件」的卡片点开；插件页有“读客户端数据”开关和风险提示；
// 关掉它写的是设置 plugins.lol.sources.client，游戏配置页里插件给的小节和“游戏事件”跟着变；
// 「插件」里有它的卡片（卡片上的开关只开关插件），插件整个关掉时 App 里的数据开关变灰。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";

/** 插件的 App（ADR 038）：左侧栏「插件」→ 点它的卡片；左侧栏亮的是「插件」。
 * （和 apps/desktop/e2e/nav.ts 的 openPlugin 一样：这个目录的测试按 CommonJS 编译，导入不了那边的 ES 模块。） */
async function openPlugin(page: Page, id: string) {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId(`plugins-card-${id}-open`).click();
  await expect(page.locator(`[data-testid="plugin-page"][data-plugin-id="${id}"]`)).toBeVisible();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
}

/** 截图放在仓库的 target/plugin-shots/step1/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step1", `${name}.png`);

async function open(page: Page) {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

/** 页面里的设置（字符串形式求值：这个目录的测试按 CommonJS 编译，函数里的 import() 会被改写）。 */
async function settings(page: Page): Promise<{ plugins?: Record<string, { enabled?: boolean; sources?: Record<string, boolean> }> }> {
  return page.evaluate("import('/src/lib/ipc.ts').then((ipc) => ipc.getSettings())");
}

test("the League of Legends plugin opens from 插件 with the client-data switch and risk note", async ({ page }, testInfo) => {
  await open(page);
  await expect(page.getByTestId("app-sidebar")).not.toContainText("英雄联盟");
  await openPlugin(page, "lol");
  const pluginPage = page.getByTestId("plugin-page-lol");
  await expect(pluginPage.getByRole("heading", { name: "英雄联盟", exact: true })).toBeVisible();
  // 开着时先是“这局”（第 2 步）；开关在“设置”
  await expect(pluginPage.getByTestId("plugin-lol-tab-match")).toHaveAttribute("aria-current", "page");
  await pluginPage.getByTestId("plugin-lol-tab-settings").click();
  await expect(pluginPage.getByTestId("plugin-lol-tab-settings")).toHaveAttribute("aria-current", "page");
  const enabled = pluginPage.getByTestId("plugin-lol-enabled");
  const client = pluginPage.getByTestId("plugin-lol-source-client");
  await expect(enabled).toHaveAttribute("aria-checked", "true");
  await expect(client).toHaveAttribute("aria-checked", "true");
  await expect(pluginPage.getByTestId("plugin-lol-source-client-risk")).toContainText("可能违反腾讯的用户协议");
  await expect(pluginPage).toContainText("读客户端数据");
  await page.screenshot({ path: shot(testInfo, "lol-plugin-page") });

  // 关掉“读客户端数据”：写进插件的设置
  await client.click();
  await expect(client).toHaveAttribute("aria-checked", "false");
  expect((await settings(page)).plugins?.lol?.sources?.client).toBe(false);

  // 游戏配置页：英雄联盟那一节是插件给的，开关是同一个；游戏事件只剩通用的
  await page.getByTestId("settings-nav-game").click();
  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const section = lol.getByTestId("game-profile-lol-data");
  await expect(section).toContainText("英雄联盟插件");
  await expect(section.getByTestId("profile-lol-data")).toHaveAttribute("aria-checked", "false");
  const events = lol.getByTestId("game-profile-events");
  await expect(events.getByTestId("game-profile-events-list")).not.toContainText("拿了大龙");
  await expect(events.getByTestId("game-profile-events-off")).toContainText("读客户端数据");
  // 在游戏配置页打开回来，插件页上也变了
  await section.getByTestId("profile-lol-data").click();
  await expect(section.getByTestId("profile-lol-data")).toHaveAttribute("aria-checked", "true");
  expect((await settings(page)).plugins?.lol?.sources?.client).toBe(true);
  await expect(events.getByTestId("game-profile-events-list")).toContainText("拿了大龙");
  await section.getByTestId("game-profile-lol-open").click();
  await page.getByTestId("plugin-page-lol").getByTestId("plugin-lol-tab-settings").click();
  await expect(page.getByTestId("plugin-page-lol").getByTestId("plugin-lol-source-client")).toHaveAttribute("aria-checked", "true");
});

test("插件 lists the plugin as a card; turning the whole plugin off greys out its data switch", async ({ page }, testInfo) => {
  await open(page);
  await page.getByTestId("settings-nav-plugins").click();
  const card = page.getByTestId("plugins-card-lol");
  await expect(card).toContainText("英雄联盟");
  await page.screenshot({ path: shot(testInfo, "settings-plugins") });

  // 卡片上的开关：只关插件，不进 App
  await card.getByTestId("plugins-card-lol-enabled").click();
  await expect(card.getByTestId("plugins-card-lol-off")).toBeVisible();
  await expect(page.getByTestId("plugin-page")).toHaveCount(0);
  expect((await settings(page)).plugins?.lol?.enabled).toBe(false);
  // 插件关着，卡片照样能点开（App 里能把它打开；数据开关变灰）
  await card.getByTestId("plugins-card-lol-open").click();
  const pluginPage = page.getByTestId("plugin-page-lol");
  await expect(pluginPage.getByTestId("plugin-lol-source-client")).toBeDisabled();
  await expect(pluginPage.getByTestId("plugin-lol-enabled")).toHaveAttribute("aria-checked", "false");
  await pluginPage.getByTestId("plugin-lol-enabled").click();
  await expect(pluginPage.getByTestId("plugin-lol-source-client")).toBeEnabled();
  expect((await settings(page)).plugins?.lol?.enabled).toBe(true);
});

test("a settings location 插件 → 英雄联盟 opens the plugin page", async ({ page }) => {
  await open(page);
  const target = await page.evaluate(`import('/src/lib/stores/navigation.ts').then((nav) => [
    nav.parseLocation("设置 → 插件，或插件 → 英雄联盟"), nav.parseLocation("插件 → 英雄联盟"), nav.parseLocation("插件"),
  ])`);
  expect(target).toEqual([
    { page: "plugins" },
    { page: "plugin", id: "lol" },
    { page: "plugins" },
  ]);
  await page.evaluate(`import('/src/lib/stores/navigation.ts').then((nav) => nav.openLocation("插件 → 英雄联盟"))`);
  await expect(page.getByTestId("plugin-page-lol")).toBeVisible();
});
