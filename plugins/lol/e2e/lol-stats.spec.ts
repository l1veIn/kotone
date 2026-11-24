// 英雄联盟插件“用网上的统计”的页面和覆盖层（ADR 038 第 3 步；页面是插件的 MCP App，ADR 039 D5）：英雄页（峡谷：克制、
// 出装、符文、加点；海斗：最好的海克斯，aramhex-data 要署名）、强度榜点一行进英雄页、聊天里的英雄卡片、OP.GG 开关默认关且
// 写着替玩家问的 issue、关掉“用网上的统计”后英雄页说一句并给“去设置”，以及游戏上方的覆盖层（照 AramHex：选英雄的评级和备选比较、海斗三选一贴在
// 卡片上的判断、评级和依据）。数据是浏览器预览的示例（apps/desktop/src/lib/ipc/lolStatsMock.ts）。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type FrameLocator, type Locator, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";

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

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
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
  const pluginPage = page.getByTestId("plugin-page-lol");
  await expect(pluginPage.getByTestId("mcp-app-frame")).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  return app(pluginPage);
}

test("the 英雄 tab searches a champion: rift counters, builds, runes and the Tencent source; mayhem shows augments with the aramhex attribution", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await lol.getByTestId("lol-app-tab-champion").click();
  await expect(lol.getByTestId("lol-champion-empty")).toBeVisible();

  // 外号也认（刀妹 = 艾瑞莉娅），默认峡谷、上路
  await lol.getByTestId("lol-champion-search").fill("刀妹");
  await lol.getByTestId("lol-champion-search-go").click();
  const view = lol.getByTestId("lol-app-champion");
  await expect(view).toHaveAttribute("data-mode", "rift");
  await expect(lol.getByTestId("lol-champion-name")).toHaveText("刀锋舞者 艾瑞莉娅");
  await expect(lol.getByTestId("lol-champion-tier")).toContainText("T1");
  await expect(lol.getByTestId("lol-champion-bad")).toContainText("贾克斯");
  await expect(lol.getByTestId("lol-champion-good")).toContainText("斯莫德");
  await expect(lol.getByTestId("lol-champion-core")).toContainText("破败王者之刃");
  await expect(lol.getByTestId("lol-champion-runes")).toContainText("征服者");
  await expect(lol.getByTestId("lol-champion-skills")).toContainText("Q > E > W");
  await expect(lol.getByTestId("lol-champion-sources")).toContainText("腾讯 101");
  // OP.GG 默认关：没有那一块
  await expect(lol.getByTestId("lol-champion-opgg")).toHaveCount(0);
  await page.screenshot({ path: shot(testInfo, "kotone-champion-rift") });

  // 换分路
  await lol.getByTestId("lol-champion-lane-MIDDLE").click();
  await expect(lol.getByTestId("lol-champion-lane-MIDDLE")).toHaveAttribute("aria-pressed", "true");
  await expect(lol.getByTestId("lol-champion-name")).toContainText("艾瑞莉娅");

  // 点一个难打的英雄：进它的页
  await lol.getByTestId("lol-champion-bad").getByTestId("lol-champion-link").first().click();
  await expect(lol.getByTestId("lol-champion-name")).toHaveText("武器大师 贾克斯");

  // 海斗：最好的海克斯、样本少的标出来、署名带许可和仓库
  await lol.getByTestId("lol-champion-mode-mayhem").click();
  await expect(view).toHaveAttribute("data-mode", "mayhem");
  await expect(lol.getByTestId("lol-champion-mayhem-rank")).toContainText("第 31 名");
  await expect(lol.getByTestId("lol-champion-augment")).toHaveCount(6);
  await expect(lol.getByTestId("lol-champion-augment").first()).toContainText("终极九头蛇");
  await expect(lol.getByTestId("lol-champion-augment").first()).toContainText("+7.9");
  await expect(lol.getByTestId("lol-champion-augment").last()).toContainText("样本少");
  const attribution = lol.getByTestId("lol-champion-attribution");
  await expect(attribution).toContainText("github.com/czgreat/aramhex-data");
  await expect(attribution).toContainText("CC BY-NC 4.0");
  await expect(lol.getByTestId("lol-champion-sources")).toContainText("aramhex-data");
  await page.screenshot({ path: shot(testInfo, "kotone-champion-mayhem") });

  // 找不到的英雄：翻译好的一句，不清掉
  await lol.getByTestId("lol-champion-search").fill("没有这个英雄");
  await lol.getByTestId("lol-champion-search-go").click();
  await expect(lol.getByTestId("lol-app-status")).toHaveAttribute("data-status", "not_found");
  await expect(lol.getByTestId("lol-champion-name")).toContainText("贾克斯");
});

test("强度榜 in the App: rows open the champion's page in the same lane", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await lol.getByTestId("lol-app-tab-tier").click();
  const rows = lol.getByTestId("lol-app-rows");
  await expect(rows).toHaveAttribute("data-lane", "TOP");
  await expect(lol.getByTestId("lol-app-tier-row")).toHaveCount(12);
  await expect(lol.getByTestId("lol-app-tier-row").first()).toContainText("T0");
  await expect(lol.getByTestId("lol-app-sources")).toContainText("腾讯 101");
  await page.screenshot({ path: shot(testInfo, "kotone-tier") });
  const name = ((await lol.getByTestId("lol-app-tier-row").first().locator(".champ b").textContent()) ?? "").trim();
  await lol.getByTestId("lol-app-tier-row").first().click();
  await expect(lol.getByTestId("lol-app-champion")).toBeVisible();
  await expect(lol.getByTestId("lol-champion-name")).toContainText(name);
  await expect(lol.getByTestId("lol-champion-lane-TOP")).toHaveAttribute("aria-pressed", "true");
  await expect(lol.getByTestId("lol-app-tab-champion")).toHaveAttribute("aria-current", "page");
});

test("asking which champions Yasuo struggles against leaves the App as a champion card in the chat that opens the full page", async ({ page }) => {
  await open(page);
  const card = page.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/champion"]');
  await expect(card).toBeVisible();
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="lol.champion"]')).toContainText("查英雄数据");
  await card.scrollIntoViewIfNeeded();
  await expect(card.getByTestId("mcp-app-frame")).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const inline = app(card);
  await expect(inline.getByTestId("lol-champion-card-name")).toHaveText("疾风剑豪 亚索");
  await expect(inline.getByTestId("lol-app-champion")).toContainText("贾克斯");
  await expect(inline.getByTestId("lol-app-champion")).toContainText("腾讯 101");
  await card.getByTestId("plugin-card-open").click();
  const stack = page.getByTestId("right-sidebar").getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/champion");
  await expect(stack.getByTestId("mcp-app-frame")).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect(app(stack).getByTestId("lol-champion-core")).toContainText("破败王者之刃");
});

test("the three data sources: online stats on, OP.GG off by default with the permission issue linked; turning stats off makes 英雄 say so", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await page.getByTestId("plugin-settings-button").click();
  const settings = page.getByTestId("plugin-settings");
  await expect(settings.getByTestId("plugin-lol-source-client")).toHaveAttribute("aria-checked", "true");
  await expect(settings.getByTestId("plugin-lol-source-stats")).toHaveAttribute("aria-checked", "true");
  await expect(settings.getByTestId("plugin-lol-source-opgg")).toHaveAttribute("aria-checked", "false");
  await expect(settings.getByTestId("plugin-lol-source-stats-risk")).toContainText("CC BY-NC 4.0");
  const link = settings.getByTestId("plugin-lol-source-opgg-link");
  await expect(link).toHaveText("github.com/opgginc/opgg-mcp/issues/26");
  await settings.getByTestId("plugin-lol-source-opgg-row").scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot(testInfo, "kotone-settings-three-sources") });

  // 关掉“用网上的统计”：App 里的英雄页说一句、给“去设置”，点了回到设置
  await settings.getByTestId("plugin-lol-source-stats").click();
  await expect(settings.getByTestId("plugin-lol-source-stats")).toHaveAttribute("aria-checked", "false");
  await page.getByTestId("plugin-settings-button").click();
  await lol.getByTestId("lol-app-tab-champion").click();
  await lol.getByTestId("lol-champion-search").fill("亚索");
  await lol.getByTestId("lol-champion-search-go").click();
  const status = lol.getByTestId("lol-app-status");
  await expect(status).toHaveAttribute("data-status", "disabled");
  await status.getByTestId("lol-app-open-settings").click();
  await expect(page.getByTestId("plugin-page")).toHaveAttribute("data-view", "settings");
});

/** 覆盖层照设计文档第四节 4：不用金色（以前最好的那张描 #f5c451），最好的用白色墨线（暗色正文色 #ecedef）。 */
const GOLD = "rgb(245, 196, 81)";
const INK = "rgb(236, 237, 239)";

test("the overlay labels the three augment cards like AramHex (verdict, grade, basis) at the host's rects", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/#/plugin-overlay?demo=augments");
  const items = page.getByTestId("plugin-overlay-item");
  await expect(items).toHaveCount(3);
  const chips = page.getByTestId("lol-overlay-augment");
  await expect(chips).toHaveCount(3);
  // 判断：首选 / 可选 / 样本太少 · 建议刷新（AramHex 的三选一标签），配评级小牌和依据
  await expect(chips.nth(1)).toHaveAttribute("data-verdict", "best");
  await expect(chips.nth(1).getByTestId("lol-overlay-verdict")).toHaveText("首选");
  await expect(chips.nth(1)).toHaveAttribute("data-grade", "S");
  await expect(chips.nth(1)).toContainText("+7.9");
  await expect(chips.nth(1)).toContainText("57.8%");
  await expect(chips.nth(1)).toContainText("棱彩第 2/38");
  await expect(chips.nth(0)).toHaveAttribute("data-verdict", "ok");
  await expect(chips.nth(0).getByTestId("lol-overlay-verdict")).toHaveText("可选");
  await expect(chips.nth(0)).toContainText("棱彩第 5/38");
  await expect(chips.nth(2)).toHaveAttribute("data-verdict", "reroll");
  await expect(chips.nth(2).getByTestId("lol-overlay-verdict")).toHaveText("样本太少 · 建议刷新");
  await expect(chips.nth(2)).toContainText("金色第 9/41");
  await expect(chips.nth(0)).toContainText("aramhex-data");
  // 颜色：首选胜色、刷新负色；评级 S 胜色；首选那张 2 px 白色墨线，不是金色；样本少（不到 300 局）的变淡
  const color = (el: Locator) => el.evaluate((e) => getComputedStyle(e).color);
  expect(await color(chips.nth(1).getByTestId("lol-overlay-verdict"))).toBe("rgb(75, 217, 139)");
  expect(await color(chips.nth(2).getByTestId("lol-overlay-verdict"))).toBe("rgb(255, 123, 134)");
  expect(await color(chips.nth(1).locator(".lol-grade"))).toBe("rgb(75, 217, 139)");
  const ring = await chips.nth(1).evaluate((e) => getComputedStyle(e).boxShadow);
  expect(ring).toContain(INK);
  expect(ring).toContain("2px");
  for (let i = 0; i < 3; i++) {
    const styles = await chips.nth(i).evaluate((e) => [getComputedStyle(e).boxShadow, getComputedStyle(e).borderColor].join(" "));
    expect(styles).not.toContain(GOLD);
  }
  expect(await chips.nth(2).evaluate((e) => getComputedStyle(e).opacity)).toBe("0.7");
  expect(await chips.nth(1).evaluate((e) => getComputedStyle(e).opacity)).toBe("1");
  // 摆在宿主给的位置（窗口里的比例）
  const box = await items.nth(1).boundingBox();
  expect(box).not.toBeNull();
  expect(Math.round(box!.x)).toBe(Math.round(1600 * 0.43));
  expect(Math.round(box!.y)).toBe(Math.round(900 * 0.24));
  expect(Math.round(box!.width)).toBe(Math.round(1600 * 0.14));
  // 不能点（点击穿透）
  expect(await items.nth(1).evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
  // 字放得下：三行都在框里，判断没被截掉
  for (let i = 0; i < 3; i++) {
    expect(await chips.nth(i).evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
    expect(await chips.nth(i).getByTestId("lol-overlay-verdict").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  }
  await page.screenshot({ path: shot(testInfo, "overlay-augment-scores") });
});

test("the champ-select overlay grades the team and the bench like AramHex and rings the best swap", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/#/plugin-overlay?demo=champ-select");
  const panel = page.getByTestId("lol-overlay-champ-select");
  await expect(panel).toBeVisible();
  // 我方 5 个：评级小牌 + 胜率 + 第几 / 共几；第一行是我
  const rows = panel.getByTestId("lol-overlay-team-row");
  await expect(rows).toHaveCount(5);
  await expect(rows.first()).toContainText("你");
  await expect(rows.first()).toHaveAttribute("data-grade", "A");
  await expect(rows.first()).toContainText("52.1%");
  await expect(rows.first()).toContainText("第 31/172");
  expect(await rows.evaluateAll((els) => els.map((e) => e.getAttribute("data-grade")))).toEqual(["A", "S", "A", "C", "B"]);
  // 备选：按胜率排，比我高的两个标 ↑，最高的那个用白色墨线圈出来（不用金色）
  await expect(panel).toContainText("胜率比你现在的英雄高");
  const seats = panel.getByTestId("lol-overlay-bench-seat");
  await expect(seats).toHaveCount(4);
  await expect(seats.first()).toContainText("斯莫德");
  await expect(seats.first()).toHaveAttribute("data-best", "true");
  await expect(seats.first()).toContainText("↑");
  await expect(seats.nth(1)).toHaveAttribute("data-better", "true");
  await expect(seats.nth(1)).toHaveAttribute("data-best", "false");
  await expect(seats.nth(2)).toHaveAttribute("data-better", "false");
  await expect(seats.nth(2)).not.toContainText("↑");
  await expect(seats.nth(3)).toHaveAttribute("data-grade", "D");
  const ring = await seats.first().evaluate((e) => getComputedStyle(e).boxShadow);
  expect(ring).toContain(INK);
  expect(ring).not.toContain(GOLD);
  expect(await seats.nth(1).evaluate((e) => getComputedStyle(e).boxShadow)).toBe("none");
  // 评级颜色：S 胜色、A 我方蓝、C 提醒黄、D 负色
  const gradeColor = (row: Locator) => row.locator(".lol-grade").evaluate((e) => getComputedStyle(e).color);
  expect(await gradeColor(rows.nth(1))).toBe("rgb(75, 217, 139)");
  expect(await gradeColor(rows.nth(0))).toBe("rgb(110, 162, 255)");
  expect(await gradeColor(rows.nth(3))).toBe("rgb(242, 181, 61)");
  expect(await gradeColor(seats.nth(3))).toBe("rgb(255, 123, 134)");
  // 插件按行数给的高度放得下全部内容，也不横着溢出
  expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  expect(await panel.evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
  const source = panel.locator(".source");
  const [panelBox, sourceBox] = [await panel.boundingBox(), await source.boundingBox()];
  expect(sourceBox!.y + sourceBox!.height).toBeLessThanOrEqual(panelBox!.y + panelBox!.height);
  await page.screenshot({ path: shot(testInfo, "overlay-champ-select") });

  // 清空：覆盖层收起
  await page.evaluate(() => (window as unknown as { kotonePluginOverlay: (p: unknown) => void }).kotonePluginOverlay({ items: [] }));
  await expect(page.getByTestId("plugin-overlay-item")).toHaveCount(0);
});
