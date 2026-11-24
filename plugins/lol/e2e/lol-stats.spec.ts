// 英雄联盟插件“用网上的统计”的页面和覆盖层（ADR 038 第 3 步）：英雄页（峡谷：克制、出装、符文、加点；海斗：最好的海克斯，
// aramhex-data 要署名）、强度榜（分路、点进英雄）、聊天里的英雄卡片、OP.GG 开关默认关且写着替玩家问的 issue，
// 以及游戏上方的覆盖层（选英雄的胜率、海斗三选一贴在卡片上的评分）。数据是浏览器预览的示例
// （apps/desktop/src/lib/ipc/lolStatsMock.ts）。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
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

/** 截图放在仓库的 target/plugin-shots/step3/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step3", `${name}.png`);

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

async function lolPage(page: Page) {
  await openPlugin(page, "lol");
  const pluginPage = page.getByTestId("plugin-page-lol");
  await expect(pluginPage).toBeVisible();
  return pluginPage;
}

test("the 英雄 tab searches a champion: rift counters, builds, runes and the Tencent source; mayhem shows augments with the aramhex attribution", async ({ page }, testInfo) => {
  await open(page);
  const pluginPage = await lolPage(page);
  await pluginPage.getByTestId("plugin-lol-tab-champion").click();
  const stack = pluginPage.getByTestId("plugin-lol-champion");
  await expect(stack.getByTestId("lol-champion-empty")).toBeVisible();

  // 外号也认（刀妹 = 艾瑞莉娅），默认峡谷、上路
  await stack.getByTestId("lol-champion-search").fill("刀妹");
  await stack.getByTestId("lol-champion-search-go").click();
  const view = stack.getByTestId("lol-champion-page");
  await expect(view).toHaveAttribute("data-mode", "rift");
  await expect(stack.getByTestId("lol-champion-name")).toHaveText("刀锋舞者 艾瑞莉娅");
  await expect(stack.getByTestId("lol-champion-tier")).toContainText("T1");
  await expect(stack.getByTestId("lol-champion-bad")).toContainText("贾克斯");
  await expect(stack.getByTestId("lol-champion-good")).toContainText("斯莫德");
  await expect(stack.getByTestId("lol-champion-core")).toContainText("破败王者之刃");
  await expect(stack.getByTestId("lol-champion-runes")).toContainText("征服者");
  await expect(stack.getByTestId("lol-champion-skills")).toContainText("Q > E > W");
  await expect(stack.getByTestId("lol-champion-sources")).toContainText("腾讯 101");
  // OP.GG 默认关：没有那一块
  await expect(stack.getByTestId("lol-champion-opgg")).toHaveCount(0);
  await page.screenshot({ path: shot(testInfo, "champion-rift-page") });

  // 换分路
  await stack.getByTestId("lol-champion-lane-MIDDLE").click();
  await expect(stack.getByTestId("lol-champion-lane-MIDDLE")).toHaveAttribute("aria-pressed", "true");
  await expect(stack.getByTestId("lol-champion-name")).toContainText("艾瑞莉娅");

  // 点一个克制它的英雄：进它的页面，能返回
  await stack.getByTestId("lol-champion-bad").getByTestId("lol-champion-link").first().click();
  await expect(stack).toHaveAttribute("data-depth", "2");
  await expect(stack.getByTestId("lol-champion-name")).toHaveText("武器大师 贾克斯");
  await stack.getByTestId("plugin-lol-champion-back").click();
  await expect(stack).toHaveAttribute("data-depth", "1");

  // 海斗：最好的海克斯、样本少的标出来、署名带许可和仓库
  await stack.getByTestId("lol-champion-mode-mayhem").click();
  await expect(view).toHaveAttribute("data-mode", "mayhem");
  await expect(stack.getByTestId("lol-champion-mayhem-rank")).toContainText("第 31 名");
  await expect(stack.getByTestId("lol-champion-augment")).toHaveCount(6);
  await expect(stack.getByTestId("lol-champion-augment").first()).toContainText("终极九头蛇");
  await expect(stack.getByTestId("lol-champion-augment").first()).toContainText("+7.9");
  await expect(stack.getByTestId("lol-champion-augment").last()).toContainText("样本少");
  const attribution = stack.getByTestId("lol-champion-attribution");
  await expect(attribution).toContainText("github.com/czgreat/aramhex-data");
  await expect(attribution).toContainText("CC BY-NC 4.0");
  await expect(stack.getByTestId("lol-champion-sources")).toContainText("aramhex-data");
  await page.screenshot({ path: shot(testInfo, "champion-mayhem-page") });

  // 找不到的英雄：说一句，不清掉
  await stack.getByTestId("lol-champion-search").fill("没有这个英雄");
  await stack.getByTestId("lol-champion-search-go").click();
  await expect(stack.getByTestId("lol-status")).toContainText("没有这个英雄");
  await expect(stack.getByTestId("lol-champion-name")).toContainText("艾瑞莉娅");
});

test("the 强度榜 tab switches lanes and modes and opens a champion", async ({ page }, testInfo) => {
  await open(page);
  const pluginPage = await lolPage(page);
  await pluginPage.getByTestId("plugin-lol-tab-tier").click();
  const stack = pluginPage.getByTestId("plugin-lol-tier");
  const rows = stack.getByTestId("lol-tier-rows");
  await expect(rows).toHaveAttribute("data-lane", "TOP");
  await expect(stack.getByTestId("lol-tier-row")).toHaveCount(12);
  await expect(stack.getByTestId("lol-tier-row").first()).toContainText("T0");
  await expect(stack.getByTestId("lol-tier-sources")).toContainText("腾讯 101");
  await stack.getByTestId("lol-tier-lane-SUPPORT").click();
  await expect(rows).toHaveAttribute("data-lane", "SUPPORT");
  await page.screenshot({ path: shot(testInfo, "tier-page") });

  // 点一个英雄：进它的英雄页（这一路）
  const first = stack.getByTestId("lol-tier-row").first();
  const name = (await first.locator("span.font-medium").textContent())?.trim() ?? "";
  await first.click();
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/champion");
  await expect(stack.getByTestId("lol-champion-name")).toContainText(name);
  await expect(stack.getByTestId("lol-champion-lane-SUPPORT")).toHaveAttribute("aria-pressed", "true");
  await stack.getByTestId("plugin-lol-tier-back").click();

  // 海斗英雄榜
  await stack.getByTestId("lol-tier-mode-mayhem").click();
  await expect(stack.getByTestId("lol-tier-page")).toHaveAttribute("data-mode", "mayhem");
  await expect(stack.getByTestId("lol-tier-row")).toHaveCount(15);
});

test("asking which champions Yasuo struggles against leaves a champion card in the chat that opens the full page", async ({ page }) => {
  await open(page);
  const card = page.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/champion"]');
  await expect(card).toBeVisible();
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="lol.champion"]')).toContainText("查英雄数据");
  await expect(card.getByTestId("lol-champion-card")).toContainText("疾风剑豪 亚索");
  await expect(card.getByTestId("lol-champion-card")).toContainText("贾克斯");
  await expect(card.getByTestId("lol-champion-card-sources")).toContainText("腾讯 101");
  await card.getByTestId("plugin-card-open").click();
  const stack = page.getByTestId("right-sidebar").getByTestId("sidebar-plugin-page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/champion");
  await expect(stack.getByTestId("lol-snapshot")).toBeVisible();
  await expect(stack.getByTestId("lol-champion-core")).toContainText("破败王者之刃");
});

test("the three data sources: online stats on, OP.GG off by default with the permission issue linked; turning stats off blocks its tabs", async ({ page }, testInfo) => {
  await open(page);
  const pluginPage = await lolPage(page);
  await pluginPage.getByTestId("plugin-lol-tab-settings").click();
  await expect(pluginPage.getByTestId("plugin-lol-source-client")).toHaveAttribute("aria-checked", "true");
  await expect(pluginPage.getByTestId("plugin-lol-source-stats")).toHaveAttribute("aria-checked", "true");
  await expect(pluginPage.getByTestId("plugin-lol-source-opgg")).toHaveAttribute("aria-checked", "false");
  await expect(pluginPage.getByTestId("plugin-lol-source-stats-risk")).toContainText("CC BY-NC 4.0");
  const link = pluginPage.getByTestId("plugin-lol-source-opgg-link");
  await expect(link).toHaveText("github.com/opgginc/opgg-mcp/issues/26");
  await pluginPage.getByTestId("plugin-lol-source-opgg-row").scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot(testInfo, "settings-three-sources") });

  // 关掉“用网上的统计”：英雄、强度榜两页只写一句、给“去设置”
  await pluginPage.getByTestId("plugin-lol-source-stats").click();
  await expect(pluginPage.getByTestId("plugin-lol-source-stats")).toHaveAttribute("aria-checked", "false");
  await pluginPage.getByTestId("plugin-lol-tab-champion").click();
  await expect(pluginPage.getByTestId("plugin-lol-source-off")).toContainText("用网上的统计");
  await pluginPage.getByTestId("plugin-lol-source-off").getByRole("button").click();
  await expect(pluginPage.getByTestId("plugin-lol-tab-settings")).toHaveAttribute("aria-current", "page");
});

test("the overlay draws the augment scores on the three cards and the champ-select panel at the host's rects", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/#/plugin-overlay?demo=augments");
  const items = page.getByTestId("plugin-overlay-item");
  await expect(items).toHaveCount(3);
  const chips = page.getByTestId("lol-overlay-augment");
  await expect(chips).toHaveCount(3);
  await expect(chips.nth(1)).toHaveAttribute("data-best", "true");
  await expect(chips.nth(0)).toHaveAttribute("data-best", "false");
  await expect(chips.nth(1)).toContainText("+7.9");
  await expect(chips.nth(1)).toContainText("最好");
  await expect(chips.nth(2)).toContainText("样本少");
  await expect(chips.nth(0)).toContainText("aramhex-data");
  // 摆在宿主给的位置（窗口里的比例）
  const box = await items.nth(1).boundingBox();
  expect(box).not.toBeNull();
  expect(Math.round(box!.x)).toBe(Math.round(1600 * 0.43));
  expect(Math.round(box!.y)).toBe(Math.round(900 * 0.27));
  expect(Math.round(box!.width)).toBe(Math.round(1600 * 0.14));
  // 不能点（点击穿透）
  expect(await items.nth(1).evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
  // 字放得下：没有溢出
  expect(await chips.nth(0).evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
  await page.screenshot({ path: shot(testInfo, "overlay-augment-scores") });

  // 换成选英雄：右上角一块，5 个人 + 备选
  await page.evaluate(() => (window as unknown as { kotonePluginOverlay: (p: unknown) => void }).kotonePluginOverlay({ items: [] }));
  await expect(items).toHaveCount(0);
  await page.goto("/#/plugin-overlay?demo=champ-select");
  await page.reload();
  const panel = page.getByTestId("lol-overlay-champ-select");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("lol-overlay-team-row")).toHaveCount(5);
  await expect(panel.getByTestId("lol-overlay-team-row").first()).toContainText("你");
  await expect(panel.getByTestId("lol-overlay-bench")).toContainText("斯莫德");
  expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: shot(testInfo, "overlay-champ-select") });
});
