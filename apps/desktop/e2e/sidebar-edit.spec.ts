// 左侧栏可以自己排（ADR 034 第六节）：组内上移 / 下移、隐藏、全部页面里打开藏起来的页面、恢复默认；
// 存在设置 ui.sidebar（预览里模拟后端记在 localStorage，刷新后还在）。“设置”不能藏。
import { expect, test, type Page, type TestInfo } from "@playwright/test";

const SHOTS = process.env.KOTONE_MUSIC_SHOTS;
const shot = (testInfo: TestInfo, name: string) => (SHOTS ? `${SHOTS}/${name}.png` : testInfo.outputPath(`${name}.png`));

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
  expect(await navOrder(page)).toEqual(["assistant", "game", "automation", "music", "general", "plugin:lol", "plugin:game-voice"]);

  await page.getByTestId("sidebar-all-pages").click();
  const dialog = page.getByTestId("sidebar-pages-dialog");
  await expect(dialog).toBeVisible();
  // 设置固定：没有隐藏按钮
  const settings = dialog.locator('[data-testid="sidebar-page-fixed"][data-page-id="advanced"]');
  await expect(settings).toBeVisible();
  await expect(settings.getByTestId("sidebar-page-toggle")).toHaveCount(0);
  // 每组第一项不能再上移
  await expect(row(page, "assistant").getByTestId("sidebar-page-up")).toBeDisabled();

  await row(page, "music").getByTestId("sidebar-page-up").click();
  await row(page, "music").getByTestId("sidebar-page-up").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "music", "game", "automation", "general", "plugin:lol", "plugin:game-voice"]);
  await row(page, "game").getByTestId("sidebar-page-toggle").click();
  await expect(row(page, "game")).toHaveAttribute("data-hidden", "true");
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "music", "automation", "general", "plugin:lol", "plugin:game-voice"]);
  await page.screenshot({ path: shot(testInfo, "sidebar-pages") });
  await dialog.getByTestId("sidebar-pages-done").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("sidebar-hidden-count")).toHaveText("1");

  // 刷新后还是这样
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "music", "automation", "general", "plugin:lol", "plugin:game-voice"]);

  // 藏起来的页面从全部页面里还能打开
  await page.getByTestId("sidebar-all-pages").click();
  await row(page, "game").getByTestId("sidebar-page-open").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("settings-nav-game")).toHaveCount(0);
  await expect(page.locator('[data-testid^="game-profile-"]').first()).toBeVisible();

  // 恢复默认
  await page.getByTestId("sidebar-all-pages").click();
  await dialog.getByTestId("sidebar-pages-reset").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "music", "general", "plugin:lol", "plugin:game-voice"]);
  await expect(dialog.getByTestId("sidebar-pages-reset")).toBeDisabled();
});

test("sidebar pages: right-click a page to move or hide it; the rail has 全部页面 too", async ({ page }, testInfo) => {
  await open(page);
  await page.getByTestId("settings-nav-automation").click({ button: "right" });
  const menu = page.getByTestId("sidebar-context-menu");
  await expect(menu).toBeVisible();
  await page.screenshot({ path: shot(testInfo, "sidebar-context-menu") });
  await menu.getByTestId("sidebar-menu-down").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "music", "automation", "general", "plugin:lol", "plugin:game-voice"]);
  // 每组最后一项不能再下移
  await page.getByTestId("settings-nav-automation").click({ button: "right" });
  await expect(page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-down")).toHaveAttribute("data-disabled", "");
  await page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-hide").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "music", "general", "plugin:lol", "plugin:game-voice"]);
  // 语音输入整组都藏了：组名也不显示
  await page.getByTestId("settings-nav-general").click({ button: "right" });
  await page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-hide").click();
  await expect(page.locator('[data-testid^="sidebar-group-"]', { hasText: "语音输入" })).toHaveCount(0);
  // 插件组（ADR 038）接上来
  await expect(page.getByTestId("sidebar-group-1")).toHaveText("插件");
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "music", "plugin:lol", "plugin:game-voice"]);

  // 图标栏：底部也有“⋯”
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.getByTestId("app-sidebar")).toHaveAttribute("data-rail", "true");
  await page.getByTestId("sidebar-all-pages").click();
  await expect(page.getByTestId("sidebar-pages-dialog")).toBeVisible();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-reset").click();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-done").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "music", "general", "plugin:lol", "plugin:game-voice"]);
  await page.getByTestId("sidebar-toggle").click();
});

test("“去看看：左侧栏 → ⋯” and “音乐” locations open the editor and the music page", async ({ page }) => {
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
  expect(await go("音乐")).toEqual({ page: "music" });
  await expect(page.getByTestId("music-page")).toBeVisible();
  expect(await go("设置 → 权限 → 放歌")).toEqual({ page: "music" });
});
