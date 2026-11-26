// 英雄联盟插件“用网上的统计”的页面（ADR 038 第 3 步；页面是插件的 MCP App，ADR 039 D5）：英雄页（峡谷：克制、
// 出装、符文、加点；海斗：最好的海克斯，aramhex-data 要署名）、强度榜点一行进英雄页、聊天里的英雄卡片、每块数据的来源标签
// 能换来源并记住（ADR 043 D8）、OP.GG 开关默认开（D9）、关掉“用网上的统计”后英雄页说一句并给“去设置”。原来的两个覆盖层改成了推送（ADR 043 D7），
// 测试在 lol-pushes.spec.ts。数据是浏览器预览的示例（apps/desktop/src/lib/ipc/lolStatsMock.ts）。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
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
  // 来源标签（ADR 043 D8）：峡谷的英雄数据默认用腾讯 101（国服）；一块只用一个来源，没有 OP.GG 那一块
  await expect(lol.getByTestId("lol-champion-source").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "qq101");
  await expect(lol.getByTestId("lol-champion-source")).toContainText("腾讯 101");
  await expect(lol.getByTestId("lol-champion-opgg")).toHaveCount(0);
  await screenshot(page, { path: shot(testInfo, "kotone-champion-rift") });

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
  // 海克斯那一块的来源标签：aramhex-data，带 CC BY-NC 4.0 的署名链接
  const augmentsTag = lol.getByTestId("lol-champion-augments-source").getByTestId("lol-source-tag");
  await expect(augmentsTag).toHaveAttribute("data-source", "aramhex");
  await expect(augmentsTag.getByTestId("lol-source-link")).toHaveText("CC BY-NC 4.0");
  await screenshot(page, { path: shot(testInfo, "kotone-champion-mayhem") });

  // 找不到的英雄：翻译好的一句，不清掉
  await lol.getByTestId("lol-champion-search").fill("没有这个英雄");
  await lol.getByTestId("lol-champion-search-go").click();
  await expect(lol.getByTestId("lol-app-status")).toHaveAttribute("data-status", "not_found");
  await expect(lol.getByTestId("lol-champion-name")).toContainText("贾克斯");
});

/** 页面里的设置（字符串形式求值：这个目录的测试按 CommonJS 编译，函数里的 import() 会被改写）。 */
async function settingsOf(page: Page): Promise<{ plugins?: Record<string, { settings?: { sourceFor?: Record<string, string> } }> }> {
  return page.evaluate("import('/src/lib/ipc.ts').then((ipc) => ipc.getSettings())");
}
/** 这一轮（ADR 043 A 轮）给主会话看的截图：target/plugin-shots/feed-and-orb/。 */
const feedShot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "feed-and-orb", `${name}.png`);

// ADR 043 D8：每块数据一个小来源标签，它同时是开关：点开选另一个来源（有的才能选），选了记在 plugins.lol.settings.sourceFor。
// 一块一次只用一个来源：换成 OP.GG 后那块是 OP.GG 原样的结果，没有腾讯 101 的数字。
test("source tags: each block says where its data comes from; the tag switches the source and the choice is remembered", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await lol.getByTestId("lol-app-tab-champion").click();
  await lol.getByTestId("lol-champion-search").fill("刀妹");
  await lol.getByTestId("lol-champion-search-go").click();
  const tag = lol.getByTestId("lol-champion-source").getByTestId("lol-source-tag");
  await expect(tag).toHaveAttribute("data-source", "qq101");
  await expect(tag).toContainText("国服");

  // 点标签：菜单里两个来源（腾讯 101 ✓、OP.GG）
  await tag.getByTestId("lol-source-switch").click();
  const menu = lol.getByTestId("lol-source-menu");
  await expect(menu).toContainText("峡谷的英雄数据用哪个来源");
  await expect(menu.getByTestId("lol-source-option")).toHaveCount(2);
  await expect(menu.locator('[data-source="qq101"]')).toHaveAttribute("aria-checked", "true");
  await screenshot(page, { path: feedShot(testInfo, "source-tag-menu") });
  await menu.locator('[data-source="opgg"]').click();
  await expect(tag).toHaveAttribute("data-source", "opgg");
  await expect(tag).toContainText("OP.GG");
  await expect(tag.getByTestId("lol-source-link")).toHaveText("op.gg");
  await expect(lol.getByTestId("lol-champion-opgg")).toContainText("class");
  await expect(lol.getByTestId("lol-champion-core")).toHaveCount(0);
  await expect(lol.getByTestId("lol-champion-tier")).toHaveCount(0);
  await expect.poll(async () => (await settingsOf(page)).plugins?.lol?.settings?.sourceFor?.rift).toBe("opgg");
  await screenshot(page, { path: feedShot(testInfo, "source-tag-opgg") });
  // 换分路照旧用 OP.GG
  await lol.getByTestId("lol-champion-lane-MIDDLE").click();
  await expect(lol.getByTestId("lol-champion-lane-MIDDLE")).toHaveAttribute("aria-pressed", "true");
  await expect(tag).toHaveAttribute("data-source", "opgg");

  // 强度榜只有腾讯 101：标签只写出处，不能换
  await lol.getByTestId("lol-app-tab-tier").click();
  const tierTag = lol.getByTestId("lol-app-sources").getByTestId("lol-source-tag");
  await expect(tierTag).toHaveAttribute("data-source", "qq101");
  await expect(tierTag.getByTestId("lol-source-switch")).toHaveCount(0);

  // 记住了：回到英雄页再查，还是 OP.GG
  await lol.getByTestId("lol-app-tab-champion").click();
  await lol.getByTestId("lol-champion-search").fill("亚索");
  await lol.getByTestId("lol-champion-search-go").click();
  await expect(lol.getByTestId("lol-champion-name")).toContainText("亚索");
  await expect(lol.getByTestId("lol-champion-source").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "opgg");

  // 海斗的海克斯：aramhex-data ↔ OP.GG，和峡谷的各记各的；换回来是 aramhex-data 的表
  await lol.getByTestId("lol-champion-mode-mayhem").click();
  const augments = lol.getByTestId("lol-champion-augments-source").getByTestId("lol-source-tag");
  await expect(augments).toHaveAttribute("data-source", "aramhex");
  await augments.getByTestId("lol-source-switch").click();
  await lol.getByTestId("lol-source-menu").locator('[data-source="opgg"]').click();
  await expect(augments).toHaveAttribute("data-source", "opgg");
  await expect(lol.getByTestId("lol-champion-augments-opgg")).toContainText("ARAM augments");
  await expect(lol.getByTestId("lol-champion-augment")).toHaveCount(0);
  await expect.poll(async () => (await settingsOf(page)).plugins?.lol?.settings?.sourceFor).toEqual({ rift: "opgg", mayhemAugments: "opgg" });
  await augments.getByTestId("lol-source-switch").click();
  await lol.getByTestId("lol-source-menu").locator('[data-source="aramhex"]').click();
  await expect(augments).toHaveAttribute("data-source", "aramhex");
  await expect(lol.getByTestId("lol-champion-augment")).toHaveCount(6);
  // 菜单点别处就关；页面上没有风险提示
  await augments.getByTestId("lol-source-switch").click();
  await expect(lol.getByTestId("lol-source-menu")).toBeVisible();
  await lol.getByTestId("lol-champion-mayhem-rank").click();
  await expect(lol.getByTestId("lol-source-menu")).toHaveCount(0);
  await expect(lol.getByTestId("lol-app-champion")).not.toContainText("风险");
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
  await screenshot(page, { path: shot(testInfo, "kotone-tier") });
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

test("the three data sources: online stats on, OP.GG on by default (ADR 043 D9); turning stats off makes 英雄 say so", async ({ page }, testInfo) => {
  await open(page);
  const lol = await pluginApp(page);
  await page.getByTestId("plugin-settings-button").click();
  const settings = page.getByTestId("plugin-settings");
  await expect(settings.getByTestId("plugin-lol-source-client")).toHaveAttribute("aria-checked", "true");
  await expect(settings.getByTestId("plugin-lol-source-stats")).toHaveAttribute("aria-checked", "true");
  // OP.GG 默认开（玩家决定“直接用”，ADR 043 D9）
  await expect(settings.getByTestId("plugin-lol-source-opgg")).toHaveAttribute("aria-checked", "true");
  // 许可（CC BY-NC 4.0）和替玩家问 OP.GG 的 issue 在“风险和许可”弹窗里（ADR 043 D8，e2e/plugin-risk.spec.ts）
  await expect(settings.getByTestId("plugin-lol-source-stats-risk")).toHaveCount(0);
  await expect(settings.getByTestId("plugin-lol-source-opgg-link")).toHaveCount(0);
  await settings.getByTestId("plugin-lol-source-opgg-row").scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "kotone-settings-three-sources") });

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
