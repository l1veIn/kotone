// 左侧栏可以自己排（ADR 034 第六节）：组内上移 / 下移、隐藏、全部页面里打开藏起来的页面、恢复默认；
// 存在设置 ui.sidebar（预览里模拟后端记在 localStorage，刷新后还在）。“设置”不能藏。
// 插件不在左侧栏（ADR 038，2026-11-24 玩家定的）：都从「设置 → 插件」点开，旧配置里记的插件页不会冒出来。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { openPlugin } from "./nav";

const SHOTS = process.env.KOTONE_MUSIC_SHOTS;
const shot = (testInfo: TestInfo, name: string) => (SHOTS ? `${SHOTS}/${name}.png` : testInfo.outputPath(`${name}.png`));
/** 插件重构第 4 步的截图（实现记录用）：仓库的 target/plugin-shots/step4/；testDir 是 apps/desktop/e2e。 */
const step4 = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "..", "..", "target", "plugin-shots", "step4", `${name}.png`);

async function navOrder(page: Page): Promise<string[]> {
  return page.getByTestId("app-sidebar").locator('nav [data-testid^="settings-nav-"]').evaluateAll(
    (els) => els.map((el) => (el.getAttribute("data-testid") ?? "").replace("settings-nav-", "")));
}

async function open(page: Page) {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(() => localStorage.removeItem("kotone.mock.sidebar"));
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

const row = (page: Page, id: string) => page.getByTestId("sidebar-pages-dialog").locator(`[data-testid="sidebar-page-row"][data-page-id="${id}"]`);

test("sidebar pages: move and hide in 全部页面, kept after a reload, hidden pages still open, restore defaults", async ({ page }, testInfo) => {
  await open(page);
  expect(await navOrder(page)).toEqual(["assistant", "game", "automation", "general"]);

  await page.getByTestId("sidebar-all-pages").click();
  const dialog = page.getByTestId("sidebar-pages-dialog");
  await expect(dialog).toBeVisible();
  // 设置固定：没有隐藏按钮
  const settings = dialog.locator('[data-testid="sidebar-page-fixed"][data-page-id="advanced"]');
  await expect(settings).toBeVisible();
  await expect(settings.getByTestId("sidebar-page-toggle")).toHaveCount(0);
  // 每组第一项不能再上移
  await expect(row(page, "assistant").getByTestId("sidebar-page-up")).toBeDisabled();

  // 全部页面里没有插件（插件在「设置 → 插件」）
  await expect(dialog.locator('[data-page-id^="plugin:"]')).toHaveCount(0);
  await row(page, "automation").getByTestId("sidebar-page-up").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "game", "general"]);
  await row(page, "game").getByTestId("sidebar-page-toggle").click();
  await expect(row(page, "game")).toHaveAttribute("data-hidden", "true");
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "general"]);
  await page.screenshot({ path: shot(testInfo, "sidebar-pages") });
  await dialog.getByTestId("sidebar-pages-done").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("sidebar-hidden-count")).toHaveText("1");

  // 刷新后还是这样
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "general"]);

  // 藏起来的页面从全部页面里还能打开
  await page.getByTestId("sidebar-all-pages").click();
  await row(page, "game").getByTestId("sidebar-page-open").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("settings-nav-game")).toHaveCount(0);
  await expect(page.locator('[data-testid^="game-profile-"]').first()).toBeVisible();

  // 恢复默认
  await page.getByTestId("sidebar-all-pages").click();
  await dialog.getByTestId("sidebar-pages-reset").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "general"]);
  await expect(dialog.getByTestId("sidebar-pages-reset")).toBeDisabled();
});

test("sidebar pages: right-click a page to move or hide it; the rail has 全部页面 too", async ({ page }, testInfo) => {
  await open(page);
  await page.getByTestId("settings-nav-game").click({ button: "right" });
  const menu = page.getByTestId("sidebar-context-menu");
  await expect(menu).toBeVisible();
  await page.screenshot({ path: shot(testInfo, "sidebar-context-menu") });
  await menu.getByTestId("sidebar-menu-down").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "game", "general"]);
  // 每组最后一项不能再下移
  await page.getByTestId("settings-nav-game").click({ button: "right" });
  await expect(page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-down")).toHaveAttribute("data-disabled", "");
  await page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-hide").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "general"]);
  // 语音输入整组都藏了：组名也不显示
  await page.getByTestId("settings-nav-general").click({ button: "right" });
  await page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-hide").click();
  await expect(page.locator('[data-testid^="sidebar-group-"]', { hasText: "语音输入" })).toHaveCount(0);
  await expect(page.getByTestId("sidebar-group-1")).toHaveCount(0);
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation"]);

  // 图标栏：底部也有“⋯”
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.getByTestId("app-sidebar")).toHaveAttribute("data-rail", "true");
  await page.getByTestId("sidebar-all-pages").click();
  await expect(page.getByTestId("sidebar-pages-dialog")).toBeVisible();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-reset").click();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-done").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "general"]);
  await page.getByTestId("sidebar-toggle").click();
});

test("“去看看：左侧栏 → ⋯” and “音乐” locations open the editor and the music plugin's page", async ({ page }) => {
  await open(page);
  const go = (where: string) => page.evaluate(async (w) => {
    const nav = await import("/src/lib/stores/navigation.ts");
    const target = nav.parseLocation(w);
    if (target) nav.navigationRequest.set({ target, n: Date.now() });
    return target;
  }, where);
  expect(await go("左侧栏 → ⋯")).toEqual({ page: "sidebar-pages" });
  await expect(page.getByTestId("sidebar-pages-dialog")).toBeVisible();
  await page.getByTestId("sidebar-pages-done").click();
  // 音乐页成了放歌插件的页面（ADR 038 第 4 步）
  expect(await go("音乐")).toEqual({ page: "plugin", id: "music" });
  await expect(page.getByTestId("music-page")).toBeVisible();
  expect(await go("设置 → 权限 → 放歌")).toEqual({ page: "plugin", id: "music" });
  expect(await go("设置 → 插件，或插件 → 英雄联盟 / 游戏语音 / 音乐")).toEqual({ page: "advanced", tab: "plugins", anchor: undefined });
  expect(await go("设置 → 插件")).toEqual({ page: "advanced", tab: "plugins", anchor: undefined });
  expect(await go("设置 → 插件 → 英雄联盟")).toEqual({ page: "plugin", id: "lol" });
  expect(await go("设置 → 插件 → 游戏语音")).toEqual({ page: "plugin", id: "game-voice" });
});

test("plugins are not in the sidebar, even with an old saved order; 设置 → 插件 opens them and 返回插件 comes back", async ({ page }, testInfo) => {
  await open(page);
  // 旧配置记过插件页（第 4 步以前的 music、之后的 plugin:lol）：左侧栏照样没有插件
  await page.evaluate(() => localStorage.setItem("kotone.mock.sidebar", JSON.stringify({ order: ["music", "plugin:lol", "game"], hidden: ["plugin:game-voice"] })));
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("app-sidebar").locator('[data-testid^="settings-nav-plugin:"]')).toHaveCount(0);
  await expect(page.getByTestId("app-sidebar")).not.toContainText("英雄联盟");
  expect(await navOrder(page)).toEqual(["assistant", "game", "automation", "general"]);
  await expect(page.getByTestId("sidebar-hidden-count")).toHaveCount(0);

  // 设置 → 插件 → 英雄联盟（点卡片的名字那一块也行），返回插件回到列表
  const lol = await openPlugin(page, "lol");
  await expect(lol.getByTestId("plugin-page-lol")).toBeVisible();
  await page.screenshot({ path: step4(testInfo, "lol-page") });
  await lol.getByTestId("plugin-page-back").click();
  await expect(page.getByTestId("settings-plugins")).toBeVisible();
  await expect(page.getByTestId("advanced-nav-plugins")).toHaveAttribute("aria-current", "page");
  await page.screenshot({ path: step4(testInfo, "settings-plugins") });
  await page.getByTestId("settings-plugin-game-voice-header").click();
  await expect(page.locator('[data-testid="plugin-page"][data-plugin-id="game-voice"]')).toBeVisible();
  await expect(page.getByTestId("settings-nav-advanced")).toHaveAttribute("aria-current", "page");
});
