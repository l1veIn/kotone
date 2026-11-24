// 英雄联盟插件的“操作客户端”（ADR 038 第 3.5 步，照 LeagueAkari）：数据开关默认关，风险提示写着 LeagueAkari #523；
// 开了以后能设自动接受对局、自动选 / 禁英雄（排好顺序、锁不锁定、等几秒），写进设置 plugins.lol.settings。
// 浏览器预览的英雄列表来自 apps/desktop/src/lib/ipc/lolStatsMock.ts。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
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

/** 截图放在仓库的 target/plugin-shots/step3.5/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step3.5", `${name}.png`);

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await openPlugin(page, "lol");
  // 插件的设置在插件页顶栏的“设置”里（App 里不放设置）
  await page.getByTestId("plugin-settings-button").click();
  const pluginPage = page.getByTestId("plugin-settings");
  await expect(pluginPage).toBeVisible();
  return pluginPage;
}

interface LolSettings {
  sources?: Record<string, boolean>;
  settings?: {
    autoAccept?: boolean;
    acceptDelaySeconds?: number;
    pick?: { enabled?: boolean; champions?: number[]; lockIn?: boolean; delaySeconds?: number };
    ban?: { enabled?: boolean; champions?: number[] };
  };
}

async function lolConfig(page: Page): Promise<LolSettings> {
  const settings = (await page.evaluate("import('/src/lib/ipc.ts').then((ipc) => ipc.getSettings())")) as {
    plugins?: Record<string, LolSettings>;
  };
  return settings.plugins?.lol ?? {};
}

test("操作客户端 is off by default with the LeagueAkari #523 risk note; the preferences only open when it is on", async ({ page }) => {
  const pluginPage = await open(page);
  const toggle = pluginPage.getByTestId("plugin-lol-source-control");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  const risk = pluginPage.getByTestId("plugin-lol-source-control-risk");
  await expect(risk).toContainText("LeagueAkari");
  await expect(risk.getByTestId("plugin-lol-source-control-link")).toHaveText("github.com/LeagueAkari/LeagueAkari/issues/523");
  await expect(pluginPage.getByTestId("lol-control-off")).toBeVisible();
  await expect(pluginPage.getByTestId("lol-control")).toHaveCount(0);

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(pluginPage.getByTestId("lol-control")).toBeVisible();
  await expect(pluginPage.getByTestId("lol-control-off")).toHaveCount(0);
  expect((await lolConfig(page)).sources?.control).toBe(true);
  // 偏好都还是关的：列表、锁定、等几秒都灰着
  await expect(pluginPage.getByTestId("lol-control-auto-accept")).toHaveAttribute("aria-checked", "false");
  await expect(pluginPage.getByTestId("lol-control-accept-delay")).toBeDisabled();
  await expect(pluginPage.getByTestId("lol-control-pick-champions-search")).toBeDisabled();
  await expect(pluginPage.getByTestId("lol-control-note")).toContainText("LeagueAkari");
});

test("auto-accept, an ordered pick list with lock-in, and a ban list are written to plugins.lol.settings", async ({ page }, testInfo) => {
  const pluginPage = await open(page);
  await pluginPage.getByTestId("plugin-lol-source-control").click();
  await expect(pluginPage.getByTestId("lol-control")).toBeVisible();

  // 自动接受，等 3 秒（超过 8 秒的夹到 8）
  await pluginPage.getByTestId("lol-control-auto-accept").click();
  const delay = pluginPage.getByTestId("lol-control-accept-delay");
  await expect(delay).toBeEnabled();
  await delay.fill("3");
  await delay.press("Enter");
  await delay.blur();
  await expect.poll(async () => (await lolConfig(page)).settings?.acceptDelaySeconds).toBe(3);
  await delay.fill("99");
  await delay.blur();
  await expect.poll(async () => (await lolConfig(page)).settings?.acceptDelaySeconds).toBe(8);
  expect((await lolConfig(page)).settings?.autoAccept).toBe(true);

  // 自动选：搜着加两个（亚索、艾瑞莉娅），把艾瑞莉娅挪到最前面，打开锁定
  const pick = pluginPage.getByTestId("lol-control-pick");
  await pluginPage.getByTestId("lol-control-pick-enabled").click();
  const search = pick.getByTestId("lol-control-pick-champions-search");
  await search.fill("亚索");
  await pick.locator('[data-testid="lol-control-pick-champions-add"][data-champion="157"]').click();
  await search.fill("艾瑞");
  await pick.locator('[data-testid="lol-control-pick-champions-add"][data-champion="39"]').click();
  const items = pick.getByTestId("lol-control-pick-champions-item");
  await expect(items).toHaveCount(2);
  await expect(items.nth(0)).toContainText("亚索");
  await expect.poll(async () => (await lolConfig(page)).settings?.pick?.champions).toEqual([157, 39]);
  await items.nth(1).getByTestId("lol-control-pick-champions-up").click();
  await expect(items.nth(0)).toContainText("艾瑞莉娅");
  await expect.poll(async () => (await lolConfig(page)).settings?.pick?.champions).toEqual([39, 157]);
  // 已经在列表里的不会再出现在候选里
  await search.fill("艾瑞");
  await expect(pick.locator('[data-testid="lol-control-pick-champions-add"][data-champion="39"]')).toHaveCount(0);
  await search.fill("");
  await pluginPage.getByTestId("lol-control-pick-lock").click();
  await expect.poll(async () => (await lolConfig(page)).settings?.pick?.lockIn).toBe(true);
  expect((await lolConfig(page)).settings?.pick?.enabled).toBe(true);

  // 自动禁：加一个劫，再移除、重新加，顺序整份替换
  const ban = pluginPage.getByTestId("lol-control-ban");
  await pluginPage.getByTestId("lol-control-ban-enabled").click();
  const banSearch = ban.getByTestId("lol-control-ban-champions-search");
  await banSearch.fill("劫");
  await ban.locator('[data-testid="lol-control-ban-champions-add"][data-champion="238"]').click();
  await banSearch.fill("盲僧");
  await ban.locator('[data-testid="lol-control-ban-champions-add"][data-champion="64"]').click();
  await expect.poll(async () => (await lolConfig(page)).settings?.ban?.champions).toEqual([238, 64]);
  await ban.getByTestId("lol-control-ban-champions-item").nth(0).getByTestId("lol-control-ban-champions-remove").click();
  await expect.poll(async () => (await lolConfig(page)).settings?.ban?.champions).toEqual([64]);

  // 别的偏好没被覆盖
  const final = (await lolConfig(page)).settings;
  expect(final?.autoAccept).toBe(true);
  expect(final?.pick).toMatchObject({ enabled: true, champions: [39, 157], lockIn: true });
  expect(final?.ban).toMatchObject({ enabled: true, champions: [64] });

  await page.getByTestId("plugin-lol-control-section").scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot(testInfo, "lol-control-settings"), fullPage: true });
});
