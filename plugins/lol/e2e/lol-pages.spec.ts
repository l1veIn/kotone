// 英雄联盟插件的页面和聊天卡片（ADR 038 第 2 步，照 MCP Apps）：她查战绩，聊天里出一张卡片（她那一刻看到的样子）；
// 点开在右侧栏是完整页面，能点进某个人的战绩、再点进某一局，能返回、关掉；她用 ui.open 也能打开插件页面和设置页；
// 从「设置 → 插件」点开的英雄联盟插件页有“这局 / 战绩 / 设置”。数据是浏览器预览的示例（apps/desktop/src/lib/ipc/lolMock.ts）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";

/** 插件的页面从「设置 → 插件」的卡片点开（左侧栏不放插件，ADR 038）；左侧栏亮的是「设置」。
 * （和 apps/desktop/e2e/nav.ts 的 openPlugin 一样：这个目录的测试按 CommonJS 编译，导入不了那边的 ES 模块。） */
async function openPlugin(page: Page, id: string) {
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-plugins").click();
  await page.getByTestId(`settings-plugin-${id}-open`).click();
  await expect(page.locator(`[data-testid="plugin-page"][data-plugin-id="${id}"]`)).toBeVisible();
  await expect(page.getByTestId("settings-nav-advanced")).toHaveAttribute("aria-current", "page");
}

/** 截图放在仓库的 target/plugin-shots/step2/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step2", `${name}.png`);

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

test("asking for this match's history leaves a card in the chat that opens the full page in the right sidebar", async ({ page }, testInfo) => {
  await open(page);
  const card = page.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]');
  await expect(card).toBeVisible();
  // 工具那一行写人话，不是 lol.scout
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="lol.scout"]')).toContainText("查这局的战绩");
  // 卡片：对面 5 个人、突破口一句、我方收成一行
  await expect(card.getByTestId("lol-match-card-enemy")).toHaveCount(5);
  await expect(card.getByTestId("lol-match-card-weak")).toContainText("突破口：亚索");
  await expect(card).toContainText("我方：");
  await card.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot(testInfo, "chat-card") });

  await card.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-tab-page")).toHaveAttribute("aria-selected", "true");
  await expect(sidebar.getByTestId("sidebar-tab-page")).toContainText("这局");
  const stack = sidebar.getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/match");
  await expect(stack).toHaveAttribute("data-depth", "1");
  // 先是她查的那一刻的样子
  await expect(stack.getByTestId("lol-snapshot")).toBeVisible();
  await expect(stack.getByTestId("lol-match-player")).toHaveCount(10);
  const weak = stack.locator('[data-testid="lol-match-player"][data-weak="true"]');
  await expect(weak).toHaveCount(1);
  await expect(weak.getByTestId("lol-match-weak")).toContainText("6 连败");
  await expect(stack.getByTestId("lol-match-hidden")).toHaveCount(1);
  await expect(stack.getByTestId("lol-match-party")).toHaveCount(4);
  await page.screenshot({ path: shot(testInfo, "sidebar-match-page") });
  // 刷新：看现在的（示例数据一样），快照的说明去掉
  await stack.getByTestId("lol-refresh").click();
  await expect(stack.getByTestId("lol-snapshot")).toHaveCount(0);

  // 点对面的亚索：他的战绩
  await weak.click();
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/player");
  await expect(stack).toHaveAttribute("data-depth", "2");
  await expect(sidebar.getByTestId("sidebar-plugin-page-title")).toHaveText("战绩");
  await expect(stack.getByTestId("lol-player-name")).toHaveText("对手子#1006");
  await expect(stack.getByTestId("lol-player-game")).toHaveCount(20);
  await page.screenshot({ path: shot(testInfo, "sidebar-player-page") });

  // 点一局：这局 10 个人
  await stack.getByTestId("lol-player-game").first().click();
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/game");
  await expect(stack).toHaveAttribute("data-depth", "3");
  await expect(stack.getByTestId("lol-game-team")).toHaveCount(2);
  await expect(stack.getByTestId("lol-game-player")).toHaveCount(10);
  await expect(stack.getByTestId("lol-game-team").first()).toHaveAttribute("data-win", "true");
  await page.screenshot({ path: shot(testInfo, "sidebar-game-page") });
  // 从单局点某个人：又是战绩
  await stack.getByTestId("lol-game-player-open").nth(2).click();
  await expect(stack).toHaveAttribute("data-depth", "4");
  await expect(stack.getByTestId("lol-player-name")).toHaveText("玩家丙#1003");

  // 返回一层层回去
  for (const depth of ["3", "2", "1"]) {
    await sidebar.getByTestId("sidebar-plugin-page-back").click();
    await expect(stack).toHaveAttribute("data-depth", depth);
  }
  await expect(sidebar.getByTestId("sidebar-plugin-page-back")).toHaveCount(0);
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/match");

  // 关掉：回到“卡片”，“页面”标签没了
  await sidebar.getByTestId("sidebar-plugin-page-close").click();
  await expect(sidebar.getByTestId("sidebar-tab-page")).toHaveCount(0);
  await expect(sidebar.getByTestId("sidebar-tab-cards")).toHaveAttribute("aria-selected", "true");
  // 卡片还在，再点开是一叠新的
  await card.getByTestId("plugin-card-body").click();
  await expect(sidebar.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-depth", "1");
});

test("ui.open opens a plugin page in the right sidebar and a settings page in settings", async ({ page }) => {
  await open(page);
  // 先离开聊天页：插件页面要回到聊天页的右侧栏
  await page.getByTestId("settings-nav-game").click();
  const opened = await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/lol/player', params: { puuid: 'sample-puuid-7' }, plugin: 'lol' })");
  expect(opened).toBe(true);
  const sidebar = page.getByTestId("right-sidebar");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const stack = sidebar.getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/player");
  // 没带数据：页面自己去读（lol.player），这个人是隐藏战绩
  await expect(stack.getByTestId("lol-player-name")).toHaveText("对手寅#1008");
  await expect(stack).toContainText("隐藏战绩");
  await expect(stack.getByTestId("lol-snapshot")).toHaveCount(0);

  // 设置页
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/settings/plugins' })")).toBe(true);
  await expect(page.getByTestId("advanced-nav-plugins")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("settings-plugin-lol")).toBeVisible();
  // 认不出的地址：不动
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/settings/' })")).toBe(false);
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'https://example.com' })")).toBe(false);
});

test("the League of Legends plugin page has 这局 and 战绩 tabs that work without her", async ({ page }, testInfo) => {
  await open(page);
  await openPlugin(page, "lol");
  const pluginPage = page.getByTestId("plugin-page-lol");
  // 默认是这局：自己去读 lol.scout
  await expect(pluginPage.getByTestId("plugin-lol-tab-match")).toHaveAttribute("aria-current", "page");
  const match = pluginPage.getByTestId("plugin-lol-match");
  await expect(match.getByTestId("lol-match-player")).toHaveCount(10);
  await expect(match.getByTestId("lol-snapshot")).toHaveCount(0);
  await match.locator('[data-testid="lol-match-player"][data-puuid="sample-puuid-0"]').click();
  await expect(match).toHaveAttribute("data-depth", "2");
  await expect(match.getByTestId("lol-player-name")).toHaveText("玩家甲#1001");

  // 战绩：默认是自己，能搜别人（名字#编号）
  await pluginPage.getByTestId("plugin-lol-tab-player").click();
  const player = pluginPage.getByTestId("plugin-lol-player");
  await expect(player.getByTestId("lol-player-name")).toHaveText("玩家甲#1001");
  await player.getByTestId("lol-player-search-input").fill("对手丑#1007");
  await player.getByTestId("lol-player-search-go").click();
  await expect(player.getByTestId("lol-player-name")).toHaveText("对手丑#1007");
  await page.screenshot({ path: shot(testInfo, "plugin-page-player-tab") });
  // 找不到的人：说一句，不清掉
  await player.getByTestId("lol-player-search-input").fill("没有这个人#0000");
  await player.getByTestId("lol-player-search-go").click();
  await expect(player.getByTestId("lol-status")).toContainText("没有这个人#0000");

  // 切回这局：刚才点到哪还在那
  await pluginPage.getByTestId("plugin-lol-tab-match").click();
  await expect(pluginPage.getByTestId("plugin-lol-match")).toHaveAttribute("data-depth", "2");
  await pluginPage.getByTestId("plugin-lol-match-back").click();
  await expect(pluginPage.getByTestId("plugin-lol-match")).toHaveAttribute("data-depth", "1");
});
