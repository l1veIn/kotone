import { expect, test, type Page } from "@playwright/test";
import { openPlugin } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 插件的“风险和许可”（ADR 043 D8）：每个插件一个弹窗，风险在前（每个有风险的数据开关）、许可和出处在后（清单的 [[notices]]）。
// 第一次打开有风险的开关、或者有风险的开关默认开着而第一次打开插件页时弹一次（记在 plugins.<id>.riskSeen）；以后从插件
// 「设置」栏上的“风险和许可”打开。只是提示：关掉弹窗不改任何开关；设置页、游戏配置页上不再一条条摆风险框。
// 浏览器预览默认当作看过了（别的测试不被弹窗挡住）；这里用 `mockRisk=unseen` 从没看过开始。

const RIOT_RULE = "“Products cannot display win rates for Augments or Arena Mode items. This applies to all websites, applications and overlays.”";

type LolConfig = { enabled?: boolean; sources?: Record<string, boolean>; riskSeen?: number };
async function lolConfig(page: Page): Promise<LolConfig> {
  return page.evaluate(() => {
    const read = (window as unknown as { __kotoneMockSettings: () => { plugins?: Record<string, LolConfig> } }).__kotoneMockSettings;
    return read().plugins?.lol ?? {};
  });
}

/** 浏览器预览的 openExternal 是 window.open：记下来，不真的开。 */
async function open(page: Page, fresh = true) {
  await page.addInitScript(() => {
    const opened: string[] = [];
    (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened = opened;
    window.open = ((url?: string | URL) => {
      opened.push(String(url));
      return null;
    }) as typeof window.open;
  });
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto(`/#/settings?onboarding=never${fresh ? "&mockRisk=unseen" : ""}`);
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}
const opened = (page: Page) => page.evaluate(() => (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened);

test("the League page shows its risks and licenses once the first time it opens (客户端 is on by default); closing changes no switch", async ({ page }, testInfo) => {
  await open(page);
  expect((await lolConfig(page)).riskSeen).toBeUndefined();
  await openPlugin(page, "lol");
  const dialog = page.getByTestId("plugin-risk-dialog-lol");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("英雄联盟：风险和许可");
  await expect(dialog).toContainText("关掉这个窗口不会改任何开关");

  // 风险在前：每个有风险的数据开关，名字、开没开、风险提示（地址能点）
  const risks = dialog.getByTestId("plugin-risk-risks");
  await expect(risks.locator("li")).toHaveCount(5);
  const client = dialog.getByTestId("plugin-risk-lol-client");
  await expect(client).toContainText("读客户端数据");
  await expect(client).toContainText("可能违反腾讯的用户协议");
  await expect(client).toHaveAttribute("data-on", "true");
  await expect(client).toContainText("开着");
  const control = dialog.getByTestId("plugin-risk-lol-control");
  await expect(control).toHaveAttribute("data-on", "false");
  await expect(control).toContainText("关着");
  await expect(control.getByTestId("plugin-risk-lol-control-link")).toHaveText("github.com/LeagueAkari/LeagueAkari/issues/523");
  await expect(dialog.getByTestId("plugin-risk-lol-chat-link")).toHaveText([
    "github.com/LeagueAkari/LeagueAkari/issues/480",
    "github.com/LeagueAkari/LeagueAkari/issues/524",
    "developer.riotgames.com/docs/lol",
  ]);
  // Riot 关于海克斯胜率的规定照原文
  const stats = dialog.getByTestId("plugin-risk-lol-stats");
  await expect(stats).toContainText(RIOT_RULE);
  await expect(stats.getByTestId("plugin-risk-lol-stats-link")).toHaveText("developer.riotgames.com/docs/lol");
  await expect(dialog.getByTestId("plugin-risk-lol-opgg-link")).toHaveText("github.com/opgginc/opgg-mcp/issues/26");

  // 许可和出处在后
  const notices = dialog.getByTestId("plugin-risk-notices");
  await expect(notices.locator("li")).toHaveCount(4);
  await expect(dialog.getByTestId("plugin-notice-lol-aramhex-data")).toContainText("CC BY-NC 4.0");
  await expect(dialog.getByTestId("plugin-notice-lol-aramhex-data-link")).toHaveText(["github.com/czgreat/aramhex-data", "creativecommons.org/licenses/by-nc/4.0"]);
  await expect(dialog.getByTestId("plugin-notice-lol-opgg")).toContainText("OP.GG 的标志和链接");
  await expect(dialog.getByTestId("plugin-notice-lol-images")).toContainText("CommunityDragon");
  await expect(dialog.getByTestId("plugin-notice-lol-riot")).toContainText("Legal Jibber Jabber");
  await screenshot(dialog, { path: testInfo.outputPath("lol-risk-dialog.png") });

  // 链接用系统浏览器打开（预览里是 window.open）
  await control.getByTestId("plugin-risk-lol-control-link").click();
  await dialog.getByTestId("plugin-notice-lol-riot-link").click();
  expect(await opened(page)).toEqual(["https://github.com/LeagueAkari/LeagueAkari/issues/523", "https://www.riotgames.com/en/legal"]);

  // 弹出来就记下看过；开关一个都没动
  await expect.poll(async () => (await lolConfig(page)).riskSeen).toBe(1);
  expect((await lolConfig(page)).sources).toBeUndefined();
  await dialog.getByTestId("plugin-risk-ok").click();
  await expect(dialog).toHaveCount(0);
  const after = await lolConfig(page);
  expect(after.sources).toBeUndefined();
  expect(after.enabled).toBeUndefined();

  // 第二次打开插件页：不再弹
  await page.getByTestId("plugin-page-back").click();
  await openPlugin(page, "lol");
  await expect(page.getByTestId("plugin-app-name")).toHaveText("英雄联盟");
  await page.waitForTimeout(300);
  await expect(page.getByTestId("plugin-risk-dialog-lol")).toHaveCount(0);
});

test("turning the plugin on (its risky switches with it) shows the dialog once; the next risky switch does not", async ({ page }) => {
  await open(page);
  // 先在「插件」的卡片上关掉英雄联盟：插件关着时它的开关都不算开，打开插件页不弹
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-lol-enabled").click();
  await expect.poll(async () => (await lolConfig(page)).enabled).toBe(false);
  await openPlugin(page, "lol");
  await page.getByTestId("plugin-settings-button").click();
  const settings = page.getByTestId("plugin-settings");
  await expect(settings.getByTestId("plugin-lol-enabled")).toHaveAttribute("aria-checked", "false");
  await page.waitForTimeout(300);
  await expect(page.getByTestId("plugin-risk-dialog-lol")).toHaveCount(0);
  expect((await lolConfig(page)).riskSeen).toBeUndefined();

  // 打开插件：读客户端数据、在客户端里发聊天这些默认开着的有风险的开关跟着开了 → 弹一次，开关照你选的
  await settings.getByTestId("plugin-lol-enabled").click();
  const dialog = page.getByTestId("plugin-risk-dialog-lol");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("plugin-risk-lol-client")).toHaveAttribute("data-on", "true");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(settings.getByTestId("plugin-lol-enabled")).toHaveAttribute("aria-checked", "true");
  expect((await lolConfig(page)).enabled).toBe(true);
  expect((await lolConfig(page)).riskSeen).toBe(1);

  // 再打开一个有风险的开关（操作客户端）：看过了，不再弹；开关照样打开
  await settings.getByTestId("plugin-lol-source-control").click();
  await expect(settings.getByTestId("plugin-lol-source-control")).toHaveAttribute("aria-checked", "true");
  await page.waitForTimeout(300);
  await expect(dialog).toHaveCount(0);
  expect((await lolConfig(page)).sources?.control).toBe(true);
});

test("the game profile's 读客户端数据 shows the dialog the first time it is turned on, not the second time", async ({ page }) => {
  await open(page);
  await page.getByTestId("settings-nav-game").click();
  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const box = lol.getByTestId("game-profile-lol-data");
  // 风险提示不摆在这里
  await expect(box).not.toContainText("可能违反腾讯的用户协议");
  await expect(box.getByTestId("game-profile-lol-risk")).toHaveCount(0);
  const toggle = box.getByTestId("profile-lol-data");
  await expect(toggle).toHaveAttribute("aria-checked", "true");

  // 关掉：不弹；再打开：弹一次，关掉弹窗后开关还是开的
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await page.waitForTimeout(300);
  await expect(page.getByTestId("plugin-risk-dialog-lol")).toHaveCount(0);
  await toggle.click();
  const dialog = page.getByTestId("plugin-risk-dialog-lol");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("plugin-risk-ok").click();
  await expect(dialog).toHaveCount(0);
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(await lolConfig(page)).toMatchObject({ sources: { client: true }, riskSeen: 1 });

  // 第二次：不弹
  await toggle.click();
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await page.waitForTimeout(300);
  await expect(dialog).toHaveCount(0);
});

test("after it was seen, 风险和许可 in the plugin's 设置 opens it; the settings have no risk boxes left", async ({ page }) => {
  await open(page, false);
  await openPlugin(page, "lol");
  // 看过了：打开插件页不弹；App 里栏上没有这个链接
  await expect(page.getByTestId("plugin-app-name")).toHaveText("英雄联盟");
  await page.waitForTimeout(300);
  await expect(page.getByTestId("plugin-risk-dialog-lol")).toHaveCount(0);
  await expect(page.getByTestId("plugin-risk-link")).toHaveCount(0);
  const before = await lolConfig(page);

  await page.getByTestId("plugin-settings-button").click();
  const settings = page.getByTestId("plugin-settings");
  // 设置里没有风险框（开关下面、操作客户端那一节）
  for (const source of ["client", "stats", "opgg", "control", "chat"]) {
    await expect(settings.getByTestId(`plugin-lol-source-${source}`)).toBeVisible();
    await expect(settings.getByTestId(`plugin-lol-source-${source}-risk`)).toHaveCount(0);
  }
  await expect(settings).not.toContainText("封号");
  await expect(settings).not.toContainText("用户协议");
  await expect(settings.getByTestId("lol-control-off")).not.toContainText("风险");

  const link = page.getByTestId("plugin-risk-link");
  await expect(link).toHaveText("风险和许可");
  await link.click();
  const dialog = page.getByTestId("plugin-risk-dialog-lol");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("plugin-risk-lol-control")).toContainText("永久封号");
  // 右上角的叉关掉：开关一个都没动
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await lolConfig(page)).toEqual(before);
  // 再点一次照样打开
  await link.click();
  await expect(dialog).toBeVisible();
});

test("a plugin without risks or licenses has no 风险和许可 and never shows the dialog", async ({ page }) => {
  await open(page);
  await openPlugin(page, "music");
  await page.waitForTimeout(300);
  await expect(page.locator('[data-testid^="plugin-risk-dialog-"]')).toHaveCount(0);
  await expect(page.getByTestId("plugin-risk-link")).toHaveCount(0);
});
