// 左侧栏可以自己排（ADR 034 第六节）：组内上移 / 下移、隐藏、全部页面里打开藏起来的页面、恢复默认；
// 存在设置 ui.sidebar（预览里模拟后端记在 localStorage，刷新后还在）。“设置”不能藏。
// 插件（ADR 038，2026-11-24 玩家定的）：左侧栏只有一项「插件」（每个插件一张卡片，点开是它的 App），
// 旧配置里记的单个插件页不会冒出来。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { openPlugin } from "./nav";
import { screenshot } from "./screenshot.cjs";

const SHOTS = process.env.KOTONE_MUSIC_SHOTS;
const shot = (testInfo: TestInfo, name: string) => (SHOTS ? `${SHOTS}/${name}.png` : testInfo.outputPath(`${name}.png`));
/** 插件页的截图（实现记录用）：仓库的 target/plugin-shots/apps/；testDir 是 apps/desktop/e2e。 */
const apps = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "..", "..", "target", "plugin-shots", "apps", `${name}.png`);
/** 第七轮的截图（插件 logo，交接第十一节）：仓库的 target/plugin-shots/round7/。 */
const round7 = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round7/${name}.png`, import.meta.url));

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
  expect(await navOrder(page)).toEqual(["assistant", "game", "automation", "plugins"]);

  await page.getByTestId("sidebar-all-pages").click();
  const dialog = page.getByTestId("sidebar-pages-dialog");
  await expect(dialog).toBeVisible();
  // 设置固定：没有隐藏按钮
  const settings = dialog.locator('[data-testid="sidebar-page-fixed"][data-page-id="advanced"]');
  await expect(settings).toBeVisible();
  await expect(settings.getByTestId("sidebar-page-toggle")).toHaveCount(0);
  // 每组第一项不能再上移
  await expect(row(page, "assistant").getByTestId("sidebar-page-up")).toBeDisabled();

  // 全部页面里插件只有一项「插件」，没有单个插件
  await expect(row(page, "plugins")).toBeVisible();
  await expect(dialog.locator('[data-page-id^="plugin:"]')).toHaveCount(0);
  await row(page, "automation").getByTestId("sidebar-page-up").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "game", "plugins"]);
  await row(page, "game").getByTestId("sidebar-page-toggle").click();
  await expect(row(page, "game")).toHaveAttribute("data-hidden", "true");
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "plugins"]);
  await screenshot(page, { path: shot(testInfo, "sidebar-pages") });
  await dialog.getByTestId("sidebar-pages-done").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("sidebar-hidden-count")).toHaveText("1");

  // 刷新后还是这样
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "plugins"]);

  // 藏起来的页面从全部页面里还能打开
  await page.getByTestId("sidebar-all-pages").click();
  await row(page, "game").getByTestId("sidebar-page-open").click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("settings-nav-game")).toHaveCount(0);
  await expect(page.locator('[data-testid^="game-profile-"]').first()).toBeVisible();

  // 恢复默认
  await page.getByTestId("sidebar-all-pages").click();
  await dialog.getByTestId("sidebar-pages-reset").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "plugins"]);
  await expect(dialog.getByTestId("sidebar-pages-reset")).toBeDisabled();
});

test("sidebar pages: right-click a page to move or hide it; the rail has 全部页面 too", async ({ page }, testInfo) => {
  await open(page);
  await page.getByTestId("settings-nav-game").click({ button: "right" });
  const menu = page.getByTestId("sidebar-context-menu");
  await expect(menu).toBeVisible();
  await screenshot(page, { path: shot(testInfo, "sidebar-context-menu") });
  await menu.getByTestId("sidebar-menu-down").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "game", "plugins"]);
  // 每组最后一项不能再下移（「插件」也和别的页面一样能排、能藏）
  await page.getByTestId("settings-nav-plugins").click({ button: "right" });
  await expect(page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-down")).toHaveAttribute("data-disabled", "");
  await page.getByTestId("sidebar-context-menu").getByTestId("sidebar-menu-hide").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "automation", "game"]);
  // 语音输入搬进了插件（ADR 038）：没有“语音输入”一组
  await expect(page.locator('[data-testid^="sidebar-group-"]', { hasText: "语音输入" })).toHaveCount(0);
  await expect(page.getByTestId("sidebar-group-1")).toHaveCount(0);

  // 图标栏：底部也有“⋯”
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.getByTestId("app-sidebar")).toHaveAttribute("data-rail", "true");
  await page.getByTestId("sidebar-all-pages").click();
  await expect(page.getByTestId("sidebar-pages-dialog")).toBeVisible();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-reset").click();
  await page.getByTestId("sidebar-pages-dialog").getByTestId("sidebar-pages-done").click();
  await expect.poll(() => navOrder(page)).toEqual(["assistant", "game", "automation", "plugins"]);
  await page.getByTestId("sidebar-toggle").click();
});

test("“去看看：左侧栏 → ⋯” and “音乐” locations open the editor and the music plugin's app", async ({ page }) => {
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
  // 插件在左侧栏的「插件」（ADR 038）；旧位置「设置 → 插件」也去那里
  expect(await go("插件")).toEqual({ page: "plugins" });
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  expect(await go("插件 → 英雄联盟")).toEqual({ page: "plugin", id: "lol" });
  // 放歌插件改名「网易云音乐」（交接第十一节第 7 条）：新名字和旧名字都认
  expect(await go("插件 → 网易云音乐")).toEqual({ page: "plugin", id: "music" });
  expect(await go("插件 → 音乐")).toEqual({ page: "plugin", id: "music" });
  expect(await go("设置 → 插件，或插件 → 英雄联盟 / 游戏语音 / 音乐")).toEqual({ page: "plugins" });
  expect(await go("设置 → 插件")).toEqual({ page: "plugins" });
  expect(await go("设置 → 插件 → 英雄联盟")).toEqual({ page: "plugin", id: "lol" });
  expect(await go("设置 → 插件 → 游戏语音")).toEqual({ page: "plugin", id: "game-voice" });
});

test("the sidebar has one 插件 page of app cards, even with an old saved order; a card opens the app full-bleed and ← 插件 comes back", async ({ page }, testInfo) => {
  await open(page);
  // 旧配置记过单个插件页（第 4 步以前的 music、之后的 plugin:lol）：左侧栏照样只有一项「插件」
  await page.evaluate(() => localStorage.setItem("kotone.mock.sidebar", JSON.stringify({ order: ["music", "plugin:lol", "game"], hidden: ["plugin:game-voice"] })));
  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("app-sidebar").locator('[data-testid^="settings-nav-plugin:"]')).toHaveCount(0);
  await expect(page.getByTestId("app-sidebar")).not.toContainText("英雄联盟");
  expect(await navOrder(page)).toEqual(["assistant", "game", "automation", "plugins"]);
  await expect(page.getByTestId("sidebar-hidden-count")).toHaveCount(0);

  // 「插件」：五张卡片（像应用启动器）；语音输入排第一（ADR 038 第 5 步）
  await page.getByTestId("settings-nav-plugins").click();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
  const grid = page.getByTestId("plugins-grid");
  for (const id of ["voice-input", "lol", "game-voice", "music", "interpreter"]) await expect(grid.getByTestId(`plugins-card-${id}`)).toBeVisible();
  // 游戏语音默认关：卡片变淡、有“已关闭”
  await expect(grid.getByTestId("plugins-card-game-voice")).toHaveAttribute("data-enabled", "false");
  await expect(grid.getByTestId("plugins-card-game-voice-off")).toBeVisible();
  await screenshot(page, { path: apps(testInfo, "plugins-page") });

  // 卡片上的开关：只开关，不进 App
  await grid.getByTestId("plugins-card-game-voice-enabled").click();
  await expect(grid.getByTestId("plugins-card-game-voice")).toHaveAttribute("data-enabled", "true");
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  await expect(page.getByTestId("plugin-page")).toHaveCount(0);
  await grid.getByTestId("plugins-card-game-voice-enabled").click();
  await expect(grid.getByTestId("plugins-card-game-voice")).toHaveAttribute("data-enabled", "false");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.plugins?.["game-voice"]?.enabled).toBe(false);

  // 点卡片进 App：占满内容区，宿主只有一条“← 插件”和名字
  const lol = await openPlugin(page, "lol");
  await expect(lol.getByTestId("plugin-app-bar")).toContainText("英雄联盟");
  await expect(lol.getByTestId("plugin-page-lol")).toBeVisible();
  await screenshot(page, { path: apps(testInfo, "lol-app") });
  await lol.getByTestId("plugin-page-back").click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");

  // 设置里没有「插件」了
  await page.getByTestId("settings-nav-advanced").click();
  await expect(page.getByTestId("advanced-nav-plugins")).toHaveCount(0);
  // 她的 ui.open 用旧地址 ui://kotone/settings/plugins：落到「插件」。玩家正在「设置」：不跳，提示上点“打开”才去
  expect(await page.evaluate(() => (window as unknown as { kotoneUiOpen: (e: unknown) => string | null }).kotoneUiOpen({ uri: "ui://kotone/settings/plugins" }))).toBe("waiting");
  await expect(page.getByTestId("advanced-page")).toBeVisible();
  await page.locator("[data-sonner-toast]").filter({ hasText: "插件" }).getByRole("button", { name: "打开" }).click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
});

test("every plugin card shows its own logo (plugins/<id>/logo.svg), greyed when off; no logo = first-letter tile", async ({ page }) => {
  // 交接第十一节玩家原话：“插件列表，同样需要每个插件有自己的 logo。”（ADR 039 D8，设计文档第五节）
  await open(page);
  await page.getByTestId("settings-nav-plugins").click();
  const grid = page.getByTestId("plugins-grid");
  const names: Record<string, string> = { "voice-input": "语音输入", lol: "英雄联盟", "game-voice": "游戏语音", music: "网易云音乐", interpreter: "同传" };
  const seen = new Set<string>();
  for (const [id, name] of Object.entries(names)) {
    const logo = grid.getByTestId(`plugins-card-${id}-logo`);
    await expect(logo).toHaveAttribute("data-logo", "file");
    // 图真的出来了（不是坏图），按卡片的 40 px 显示
    await expect.poll(() => logo.evaluate((img) => (img as HTMLImageElement).complete ? (img as HTMLImageElement).naturalWidth : 0)).toBeGreaterThan(0);
    expect((await logo.boundingBox())?.width).toBe(40);
    // 是这个插件自己的那张：SVG 里的 <title> 就是插件名（小 SVG 会被 Vite 内联成 data: 地址，大的是同源文件）
    const src = (await logo.getAttribute("src"))!;
    const data = /^data:[^,]*?(;base64)?,(.*)$/s.exec(src);
    const svg = data ? (data[1] ? Buffer.from(data[2], "base64").toString("utf8") : decodeURIComponent(data[2])) : await (await page.request.get(src)).text();
    expect(svg).toContain(`<title>${name}</title>`);
    seen.add(src);
  }
  expect(seen.size).toBe(5);
  // 游戏语音默认关：logo 变灰；开着的不灰
  await expect(grid.getByTestId("plugins-card-game-voice-logo")).toHaveClass(/grayscale/);
  await expect(grid.getByTestId("plugins-card-lol-logo")).not.toHaveClass(/grayscale/);

  // 没有 logo 的插件（以后的第三方）：名字第一个字（英文大写）+ 按 id 定的底色（8 个里挑，同一个 id 每次一样）
  const fallback = await page.evaluate(async () => {
    const logos = await import("/src/lib/pluginLogos.ts");
    return {
      lol: logos.pluginLogo("lol") !== null,
      weather: logos.pluginLogo("weather"),
      tian: logos.fallbackLogo("weather", "天气"),
      again: logos.fallbackLogo("weather", "别的名字").color,
      memory: logos.fallbackLogo("mcp-memory", "memory"),
      colors: [...logos.FALLBACK_COLORS] as string[],
    };
  });
  expect(fallback.lol).toBe(true);
  expect(fallback.weather).toBeNull();
  expect(fallback.tian.text).toBe("天");
  expect(fallback.colors).toContain(fallback.tian.color);
  expect(fallback.again).toBe(fallback.tian.color);
  expect(fallback.memory.text).toBe("M");
  expect(fallback.colors).toContain(fallback.memory.color);

  // 亮暗两种主题下的样子（logo 自带底色方块，两种主题一样）
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await screenshot(page, { path: round7(`plugins-logos-${theme}`), animations: "disabled" });
  }
});
