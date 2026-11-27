// 英雄联盟插件的页面都是插件的 MCP App（ADR 039 D5；docs/design/lol-app.md）：插件页整页是 App（自己的导航：主页 · 这局 · 战绩 ·
// 英雄 · 强度榜），聊天里的卡片是同一个 App 的 inline，右侧栏是 fullscreen。插件的设置是宿主的页面（插件页顶栏的“设置”）。
// 这里看 App 在 Kotone 里的样子和用法：这局的加载、结算、不在对局里；点人看战绩、点一局看单局详情、能返回；搜人；她 ui.open
// 带参数打开；设置按钮。数据是浏览器预览的示例（apps/desktop/src/lib/ipc/lolMock.ts、lolStatsMock.ts）；App 是
// plugins/lol/app/dist（test:e2e 先打包）。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type FrameLocator, type Locator, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

/** 插件的 App（ADR 038）：左侧栏「插件」→ 点它的卡片；左侧栏亮的是「插件」。
 * （和 apps/desktop/e2e/nav.ts 的 openPlugin 一样：这个目录的测试按 CommonJS 编译，导入不了那边的 ES 模块。） */
async function openPlugin(page: Page, id: string) {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId(`plugins-card-${id}-open`).click();
  await expect(page.locator(`[data-testid="plugin-page"][data-plugin-id="${id}"]`)).toBeVisible();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
}

/** 截图放在仓库的 target/plugin-shots/round7/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round7", `${name}.png`);
/** 这一轮（ADR 043 A 轮）给主会话看的截图：target/plugin-shots/feed-and-orb/。 */
const feedShot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "feed-and-orb", `${name}.png`);

async function open(page: Page, phase?: string) {
  await page.setViewportSize({ width: 1440, height: 900 });
  if (phase) await page.addInitScript(`window.__kotoneLolPhase = ${JSON.stringify(phase)}`);
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

/** 一个 McpAppFrame 里的 App（沙盒代理页里面那一层 iframe）。 */
function app(scope: Locator): FrameLocator {
  return scope.frameLocator('[data-testid="mcp-app-iframe"]').frameLocator("iframe");
}

/** 插件页（整页是 App）。 */
async function pluginApp(page: Page): Promise<FrameLocator> {
  await openPlugin(page, "lol");
  const frame = page.getByTestId("plugin-page-lol").getByTestId("mcp-app-frame");
  await expect(frame).toHaveAttribute("data-display", "fullscreen");
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  return app(page.getByTestId("plugin-page-lol"));
}

test("the plugin opens on the summoner home and leads into the loading match, a player and a game", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  // 宿主不再给一排分页：导航在 App 里
  await expect(page.getByTestId("plugin-lol-tab-match")).toHaveCount(0);
  await expect(lol.getByTestId("lol-app-tab-home")).toHaveAttribute("aria-current", "page");
  for (const tab of ["match", "player", "champion", "tier"]) await expect(lol.getByTestId(`lol-app-tab-${tab}`)).toBeVisible();
  await expect(lol.getByTestId("lol-app-tab-settings")).toHaveCount(0);
  await expect(lol.getByTestId("lol-home-phase")).toContainText("加载");
  await lol.getByTestId("lol-home-match").click();
  await expect(lol.getByTestId("lol-app-tab-match")).toHaveAttribute("aria-current", "page");

  // 这局（加载）：阶段牌；宽的插件页照 LeagueAkari 排（两队上下、每人一列，ADR 043 D9）；10 个人（隐藏了身份的两个
  // 照样在，只有英雄）；只给事实，没有“突破口”（D2）；隐藏战绩、开黑字母；页脚只有出处和拳头的声明，没有风险提示（D8）
  const match = lol.getByTestId("lol-app-match");
  await expect(match).toHaveAttribute("data-phase", "loading");
  await expect(lol.getByTestId("lol-match-phase")).toContainText("加载中");
  await expect(lol.getByTestId("lol-match-grid")).toBeVisible();
  await expect(lol.getByTestId("lol-match-player")).toHaveCount(10);
  await expect(lol.locator('[data-testid="lol-match-player"][data-identity="hidden"]')).toHaveCount(2);
  await expect(lol.getByTestId("lol-match-identity")).toHaveText(["隐藏了身份", "隐藏了身份"]);
  await expect(lol.getByTestId("lol-team-hidden")).toHaveText("2 人隐藏了身份");
  await expect(match).not.toContainText("突破口");
  await expect(lol.getByTestId("lol-match-hidden")).toHaveCount(1);
  await expect(lol.getByTestId("lol-match-party")).toHaveCount(4);
  await expect(lol.getByTestId("lol-app-foot")).toContainText("Legal Jibber Jabber");
  await expect(lol.getByTestId("lol-app-foot").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "client");
  await expect(lol.getByTestId("lol-app-risk")).toHaveCount(0);
  await expect(match).not.toContainText("封号");
  await screenshot(page, { path: shot(testInfo, "kotone-plugin-page-match") });
  await screenshot(page, { path: feedShot(testInfo, "plugin-page-match-grid") });

  // 点对面的亚索：他的战绩（App 里换页）；隐藏了身份的那一列点不开
  await expect(lol.locator('[data-testid="lol-match-player"][data-identity="hidden"]').first()).not.toHaveAttribute("role", "button");
  await lol.locator('[data-testid="lol-match-player"][data-puuid="sample-puuid-5"]').click();
  await expect(lol.getByTestId("lol-app-player")).toBeVisible();
  await expect(lol.getByTestId("lol-app-tab-player")).toHaveAttribute("aria-current", "page");
  await expect(lol.getByTestId("lol-player-name")).toHaveText("对手子#1006");
  await expect(lol.getByTestId("lol-player-game")).toHaveCount(20);
  await screenshot(page, { path: shot(testInfo, "kotone-plugin-page-player") });

  // 点一局：两队 10 个人；“← 战绩”回去
  await lol.getByTestId("lol-player-game").first().click();
  await expect(lol.getByTestId("lol-app-game")).toBeVisible();
  await expect(lol.getByTestId("lol-game-team")).toHaveCount(2);
  await expect(lol.getByTestId("lol-game-player")).toHaveCount(10);
  await expect(lol.getByTestId("lol-game-team").first()).toHaveAttribute("data-win", "true");
  await screenshot(page, { path: shot(testInfo, "kotone-plugin-page-game") });
  await lol.getByTestId("lol-game-back").click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("对手子#1006");
  // 从单局点某个人：他的战绩
  await lol.getByTestId("lol-player-game").first().click();
  await lol.getByTestId("lol-game-player-open").nth(2).click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("玩家丙#1003");

  // 战绩分页：搜人（名字#编号）；找不到的说一句，不清掉
  await lol.getByTestId("lol-app-tab-player").click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("玩家甲#1001");
  await lol.getByTestId("lol-player-search").fill("对手丑#1007");
  await lol.getByTestId("lol-player-search-go").click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("对手丑#1007");
  await lol.getByTestId("lol-player-search").fill("没有这个人#0000");
  await lol.getByTestId("lol-player-search-go").click();
  await expect(lol.getByTestId("lol-app-status-banner")).toContainText("没找到");
  await expect(lol.getByTestId("lol-player-name")).toHaveText("对手丑#1007");
});

test("outside a game the home shows my profile and the ten latest games", async ({ page }) => {
  await open(page, "none");
  const lol = await pluginApp(page);
  const home = lol.getByTestId("lol-app-home");
  await expect(home.getByTestId("lol-home-name")).toContainText(/玩家甲\s*#1001/);
  await expect(home.getByTestId("lol-home-background")).toBeVisible();
  await expect(home.getByTestId("lol-home-background")).toHaveAttribute("src", /irelia_splash_uncentered_0\.jpg$/);
  await expect(home).toContainText("黄金 II");
  await expect(home.getByTestId("lol-home-match")).toHaveCount(0);
  await expect(home.getByRole("heading", { name: "近期战绩" })).toBeVisible();
  await expect(home.getByTestId("lol-home-game")).toHaveCount(10);
  await expect(home.getByTestId("lol-home-page-size")).toHaveValue("10");
  await expect(home.getByTestId("lol-home-prev")).toBeDisabled();
  const recent = home.getByTestId("lol-home-game").first();
  await expect(recent).toHaveAttribute("data-game-id", "9000000");
  await expect(recent).toContainText("艾瑞莉娅");
  await recent.click();
  await expect(lol.getByTestId("lol-game-player")).toHaveCount(10);
  await expect(lol.getByTestId("lol-game-back")).toContainText("主页");
  await lol.getByTestId("lol-game-back").click();
  await expect(lol.getByTestId("lol-app-tab-home")).toHaveAttribute("aria-current", "page");
  await home.getByTestId("lol-home-history").click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("玩家甲#1001");
  await expect(lol.getByTestId("lol-player-game")).toHaveCount(20);
});

test("older history pages keep their size and position after a game detail visit", async ({ page }) => {
  await open(page, "none");
  const lol = await pluginApp(page);
  const rows = lol.getByTestId("lol-home-game");
  const size = lol.getByTestId("lol-home-page-size");
  const next = lol.getByTestId("lol-home-next");
  const prev = lol.getByTestId("lol-home-prev");
  await expect(rows).toHaveCount(10);
  await next.click();
  await expect(rows.first()).toHaveAttribute("data-game-id", "9000010");
  await size.selectOption("20");
  await expect(rows).toHaveCount(20);
  await expect(rows.first()).toHaveAttribute("data-game-id", "9000000");
  await next.click();
  await expect(rows.first()).toHaveAttribute("data-game-id", "9000020");
  await expect(rows.last()).toHaveAttribute("data-game-id", "9000039");
  await rows.nth(3).click();
  await expect(lol.getByTestId("lol-game-player")).toHaveCount(10);
  await expect(lol.getByTestId("lol-game-title")).toContainText("17:00");
  await lol.getByTestId("lol-game-back").click();
  await expect(size).toHaveValue("20");
  await expect(lol.getByTestId("lol-home-page")).toContainText("2");
  await expect(rows.first()).toHaveAttribute("data-game-id", "9000020");
  await expect(rows.last()).toHaveAttribute("data-game-id", "9000039");
  await next.click();
  await expect(rows).toHaveCount(7);
  await expect(rows.last()).toHaveAttribute("data-game-id", "9000046");
  await expect(next).toBeDisabled();
  await prev.click();
  await expect(rows.first()).toHaveAttribute("data-game-id", "9000020");
  await expect(rows).toHaveCount(20);
});

test("missing splash artwork leaves the profile and recent game usable", async ({ page }) => {
  const attempts = new Set<string>();
  await page.route("**/irelia_splash_uncentered_0.jpg", async (route) => {
    attempts.add(route.request().url());
    await route.fulfill({ status: 404, body: "" });
  });
  await open(page, "none");
  const lol = await pluginApp(page);
  await expect.poll(() => attempts.size).toBe(2);
  await expect(lol.getByTestId("lol-home-background")).toHaveCount(0);
  await expect(lol.getByTestId("lol-home-name")).toBeVisible();
  await lol.getByTestId("lol-home-game").first().click();
  await expect(lol.getByTestId("lol-game-player")).toHaveCount(10);
});

test("with client data switched off the plugin still opens on home and online stats work", async ({ page }) => {
  await open(page);
  await openPlugin(page, "lol");
  await page.getByTestId("plugin-settings-button").click();
  const client = page.getByTestId("plugin-lol-source-client");
  await client.click();
  await expect(client).toHaveAttribute("aria-checked", "false");
  const lol = await pluginApp(page);
  await expect(lol.getByTestId("lol-app-tab-home")).toHaveAttribute("aria-current", "page");
  await expect(lol.getByTestId("lol-app-home")).toContainText("数据开关");
  await expect(lol.getByTestId("lol-home-background")).toHaveCount(0);
  await expect(lol.getByTestId("lol-home-match")).toHaveCount(0);
  await lol.getByTestId("lol-app-tab-tier").click();
  await expect(lol.getByTestId("lol-app-tier-row")).toHaveCount(12);
});

test("after the game 这局 shows the result and opens this game with the 刚结束 tag; outside a game it says so", async ({ page }, testInfo) => {
  await open(page, "post_match");
  const lol = await pluginApp(page);
  await lol.getByTestId("lol-app-tab-match").click();
  await expect(lol.getByTestId("lol-app-match")).toHaveAttribute("data-phase", "post_match");
  await expect(lol.getByTestId("lol-match-phase")).toContainText("结算");
  const result = lol.getByTestId("lol-match-result");
  await expect(result).toContainText("胜利");
  await expect(result).toContainText("18:32");
  await expect(lol.getByTestId("lol-match-player")).toHaveCount(10);
  await screenshot(page, { path: shot(testInfo, "kotone-plugin-page-post-match") });
  await lol.getByTestId("lol-match-result-open").click();
  await expect(lol.getByTestId("lol-app-game")).toHaveAttribute("data-just-ended", "true");
  await expect(lol.getByTestId("lol-game-just-ended")).toHaveText("刚结束");
  await lol.getByTestId("lol-game-back").click();
  await expect(lol.getByTestId("lol-match-result")).toBeVisible();

  // 不在对局里：翻译好的一句话（不是给她的原话）+ 看我的战绩
  await page.evaluate("window.__kotoneLolPhase = 'none'");
  await lol.getByTestId("lol-app-refresh").click();
  await expect(lol.getByTestId("lol-match-phase")).toContainText("不在对局里");
  await lol.getByTestId("lol-app-tab-player").click();
  await lol.getByTestId("lol-app-tab-match").click();
  const status = lol.getByTestId("lol-app-status");
  await expect(status).toHaveAttribute("data-status", "not_in_game");
  await expect(status).toContainText("现在不在对局里");
  await expect(status).not.toContainText("dev:web");
  await status.getByTestId("lol-app-see-mine").click();
  await expect(lol.getByTestId("lol-player-name")).toHaveText("玩家甲#1001");
});

test("the 设置 button next to the plugin's name opens the host's settings for it and goes back to the App", async ({ page }, testInfo) => {
  await open(page);
  await pluginApp(page);
  const button = page.getByTestId("plugin-settings-button");
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await button.click();
  await expect(page.getByTestId("plugin-page")).toHaveAttribute("data-view", "settings");
  const settings = page.getByTestId("plugin-settings");
  await expect(settings.getByTestId("plugin-lol-source-client")).toHaveAttribute("aria-checked", "true");
  // 风险提示收进栏上的“风险和许可”（ADR 043 D8）
  await expect(settings.getByTestId("plugin-lol-source-client-risk")).toHaveCount(0);
  await expect(page.getByTestId("plugin-risk-link")).toBeVisible();
  await expect(settings.getByTestId("plugin-lol-control-section")).toBeVisible();
  await screenshot(page, { path: shot(testInfo, "kotone-plugin-settings") });
  // 点插件名或再点一次“设置”：回到 App（没重新加载，还在原来那页）
  await page.getByTestId("plugin-app-name").click();
  await expect(page.getByTestId("plugin-page")).toHaveAttribute("data-view", "app");
  await expect(page.getByTestId("plugin-settings")).toHaveCount(0);
  await expect(app(page.getByTestId("plugin-page-lol")).getByTestId("lol-app-home")).toBeVisible();
});

test("asking for this match's history leaves the App as a card in the chat; opening it shows the full App in the right sidebar", async ({ page }, testInfo) => {
  await open(page);
  const card = page.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]');
  await expect(card).toBeVisible();
  await expect(card).toHaveAttribute("data-app", "true");
  // 工具那一行写人话，不是 lol.scout
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="lol.scout"]')).toContainText("查这局的战绩");
  await card.scrollIntoViewIfNeeded();
  const frame = card.getByTestId("mcp-app-frame");
  await expect(frame).toHaveAttribute("data-display", "inline");
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const inline = app(card);
  const matchCard = inline.getByTestId("lol-match-card");
  await expect(matchCard).toBeVisible();
  // 聊天里的卡片不到 640 宽：两队上下排、每人一行（ADR 043 D9）；隐藏了身份的两行照样在；没有突破口、没有风险提示
  await expect(matchCard).toHaveAttribute("data-layout", "rows");
  await expect(inline.getByTestId("lol-card-row")).toHaveCount(10);
  await expect(inline.locator('[data-testid="lol-card-row"][data-identity="hidden"]')).toHaveCount(2);
  await expect(inline.getByTestId("lol-card-identity")).toHaveText(["隐藏了身份", "隐藏了身份"]);
  await expect(matchCard).not.toContainText("突破口");
  await expect(inline.getByTestId("lol-card-foot")).toContainText("客户端");
  await expect(inline.getByTestId("lol-app-risk")).toHaveCount(0);
  await expect(matchCard).not.toContainText("风险");
  await screenshot(card, { path: shot(testInfo, "kotone-chat-card-match") });

  await card.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-tab-page")).toHaveAttribute("aria-selected", "true");
  const stack = sidebar.getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/match");
  await expect(stack.getByTestId("mcp-app-frame")).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const full = app(stack);
  await expect(full.getByTestId("lol-app-tab-match")).toHaveAttribute("aria-current", "page");
  await expect(full.getByTestId("lol-match-player")).toHaveCount(10);
  // App 里点人：在 App 里换页（右侧栏的一叠不变）
  await full.locator('[data-testid="lol-match-player"][data-puuid="sample-puuid-5"]').click();
  await expect(full.getByTestId("lol-player-name")).toHaveText("对手子#1006");
  await expect(stack).toHaveAttribute("data-depth", "1");
  await screenshot(page, { path: shot(testInfo, "kotone-sidebar-player") });
});

// ADR 043 D9：开局战绩照 LeagueAkari 的排布，适度简化给小窗用（玩家：“窄的时候两队上下排，每人一行，点开看详情”）。
// 卡片的宽度按宿主给的来：这里把聊天里那张卡片的外框改成悬浮窗小窗的宽（360）和一个宽的宿主（720，例如 Claude 的聊天卡片）。
test("the opening card: at mini-window width each player is a row that opens in place; a wide host gets LeagueAkari's columns", async ({ page }, testInfo) => {
  await open(page);
  // 高一点的窗口：截图里卡片不被下面的输入框挡住
  await page.setViewportSize({ width: 1440, height: 1500 });
  const card = page.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]');
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByTestId("mcp-app-frame")).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const inline = app(card);
  const matchCard = inline.getByTestId("lol-match-card");
  const resize = async (width: number) => {
    await card.evaluate((el, w) => {
      (el as HTMLElement).style.width = `${w}px`;
      (el as HTMLElement).style.maxWidth = `${w}px`;
      el.scrollIntoView({ block: "start" });
    }, width);
  };

  // 悬浮窗的小窗（约 360）：两队上下排、对面在上，每人一行
  await resize(360);
  await expect(matchCard).toHaveAttribute("data-layout", "rows");
  const teams = inline.locator('[data-testid="lol-card-enemies"], [data-testid="lol-card-allies"]');
  await expect(teams).toHaveCount(2);
  await expect(teams.first()).toHaveAttribute("data-testid", "lol-card-enemies");
  await expect(inline.getByTestId("lol-card-row")).toHaveCount(10);
  // 一行里没有挤不下的东西：每一行都不比卡片宽
  const overflow = await inline.getByTestId("lol-card-row").evaluateAll((rows) => rows.filter((r) => r.scrollWidth > r.clientWidth + 1).length);
  expect(overflow).toBe(0);

  // 点一行：在下面展开详情（段位、胜负、KDA、最近 10 局、标签）；点另一行换成那一个；再点收起
  const yasuo = inline.getByTestId("lol-card-row").filter({ hasText: "亚索" });
  await yasuo.click();
  await expect(yasuo).toHaveAttribute("aria-expanded", "true");
  const detail = inline.getByTestId("lol-card-detail");
  await expect(detail).toHaveCount(1);
  await expect(detail).toContainText("KDA");
  await expect(detail).toContainText("6 连败");
  await expect(detail.locator(".recent .ci")).toHaveCount(10);
  await screenshot(card, { path: feedShot(testInfo, "opening-card-mini-360-expanded") });
  const ashe = inline.getByTestId("lol-card-row").filter({ hasText: "艾希" });
  await ashe.click();
  await expect(detail).toHaveCount(1);
  await expect(yasuo).toHaveAttribute("aria-expanded", "false");
  await ashe.click();
  await expect(detail).toHaveCount(0);
  // 隐藏了身份的那两行只有英雄，点不开
  const hidden = inline.locator('[data-testid="lol-card-row"][data-identity="hidden"]');
  await expect(hidden).toHaveCount(2);
  await expect(hidden.first()).not.toHaveAttribute("aria-expanded", /.*/);
  await expect(hidden.first()).toContainText("金克丝");
  await screenshot(card, { path: feedShot(testInfo, "opening-card-mini-360") });

  // 宽的宿主：LeagueAkari 用户习惯的样子，每队一行、每人一列（带最近 5 局）
  await resize(720);
  await expect(matchCard).toHaveAttribute("data-layout", "grid");
  await expect(inline.getByTestId("lol-card-row")).toHaveCount(10);
  await expect(inline.locator('[data-testid="lol-card-row"][data-identity="hidden"]')).toHaveCount(2);
  await expect(inline.locator('[data-testid="lol-card-row"][data-puuid="sample-puuid-5"] .games li')).toHaveCount(5);
  await expect(matchCard).not.toContainText("风险");
  await screenshot(card, { path: feedShot(testInfo, "opening-card-wide-720") });
});

test("ui.open with parameters opens that page of the App in the right sidebar; old settings addresses still land", async ({ page }) => {
  await open(page);
  // 玩家在别的页面（游戏列表）：不切过去，放在右侧栏等玩家点（共享资源评估第二节第 6 条）；点提示上的“打开”才回到聊天页的右侧栏
  await page.getByTestId("settings-nav-game").click();
  const opened = await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/lol/player', params: { puuid: 'sample-puuid-7' }, plugin: 'lol' })");
  expect(opened).toBe("waiting");
  await expect(page.getByTestId("assistant-chat")).toBeHidden();
  await page.locator("[data-sonner-toast]").filter({ hasText: "英雄联盟 · 战绩" }).getByRole("button", { name: "打开" }).click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const stack = sidebar.getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/player");
  // 带了参数没带数据：宿主替 App 调 lol.player 再推过去；这个人是隐藏战绩
  const full = app(stack);
  await expect(full.getByTestId("lol-player-name")).toHaveText("对手寅#1008");
  await expect(full.getByTestId("lol-player-hidden")).toBeVisible();

  // 旧的设置页地址：插件搬到了左侧栏的「插件」（玩家在聊天页：她带你过去）
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/settings/plugins' })")).toBe("shown");
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("plugins-card-lol")).toBeVisible();
  // 认不出的地址：不动
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/settings/' })")).toBe(null);
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'https://example.com' })")).toBe(null);
});

// 预览里的素材层是开发服务器的假代理（vite.config.ts：`/<密钥>/assets/<插件>/…` 都回一张 1×1 的 PNG），不连 CommunityDragon。
test("the 英雄 page shows skill icons with the key on each and the name on hover; letters when there is no picture", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await lol.getByTestId("lol-app-tab-champion").click();
  await lol.getByTestId("lol-champion-search").fill("刀妹");
  await lol.getByTestId("lol-champion-search-go").click();
  await expect(lol.getByTestId("lol-champion-name")).toHaveText("刀锋舞者 艾瑞莉娅");

  // 主升那一排：被动 + Q W E R，每个是技能图标、右下角压键位，技能名在鼠标停留提示里
  const skills = lol.getByTestId("lol-champion-skills");
  const top = skills.locator(".keys").getByTestId("lol-ability");
  await expect(top).toHaveCount(5);
  expect(await top.evaluateAll((els) => els.map((e) => e.getAttribute("data-key")))).toEqual(["P", "Q", "W", "E", "R"]);
  const q = top.nth(1);
  await expect(q).toHaveAttribute("data-icon", "1");
  // 图走 Kotone 的素材层（回环代理的 /assets/lol/ + 客户端路径）
  await expect(q.locator("img")).toHaveAttribute("src", /\/assets\/lol\/lol-game-data\/assets\/ASSETS\/Characters\/Irelia\/HUD\/Icons2D\/Irelia_Q\.png$/);
  await expect.poll(() => q.locator("img").evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth)).toBe(1);
  await expect(q.locator(".lv")).toHaveText("Q");
  await expect(q.locator(".ci")).toHaveAttribute("title", "Q · 利刃冲击");
  await expect(top.first().locator(".ci")).toHaveAttribute("title", "被动 · 艾欧尼亚热诚");
  // 前几级的加点顺序：每一级一个图标
  await expect(skills.locator(".order").getByTestId("lol-ability").locator("img")).toHaveCount(11);
  await skills.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "kotone-champion-skills") });

  // 没有技能资料的英雄（客户端没开、CommunityDragon 也没取到）：字母格子，没有被动那一格
  await lol.getByTestId("lol-champion-bad").getByTestId("lol-champion-link").first().click();
  await expect(lol.getByTestId("lol-champion-name")).toHaveText("武器大师 贾克斯");
  const letters = skills.locator(".keys").getByTestId("lol-ability");
  await expect(letters).toHaveCount(4);
  await expect(skills.locator("img")).toHaveCount(0);
  expect(await letters.evaluateAll((els) => els.map((e) => e.textContent?.trim()))).toEqual(["Q", "W", "E", "R"]);
});
