// CS2 插件（ADR 036 D9、ADR 047 第 3 期）：左侧栏「插件」里有它的卡片（默认开），点开是它的页面——这一局（比分、回合、你的数据、
// 炸弹倒数）和游戏的配置文件（写在哪、有没有写上、开关）；设置里是插件开关和宿主画的五条推送。没有“风险和许可”（官方接口）。
// 浏览器预览的后端是 plugins/cs2/ui/mock.ts（假的 cfg 路径，`window.__kotoneCs2Match` 切有没有在打）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

/** 截图放在仓库的 target/plugin-shots/cs2/；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "cs2", `${name}.png`);

async function openPage(page: Page) {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("settings-nav-plugins").click();
  const card = page.getByTestId("plugins-card-cs2");
  await expect(card).toContainText("CS2");
  await expect(card.getByTestId("plugins-card-cs2-enabled")).toHaveAttribute("aria-checked", "true");
  await card.getByTestId("plugins-card-cs2-open").click();
  const pluginPage = page.getByTestId("plugin-page-cs2");
  await expect(pluginPage.getByRole("heading", { name: "CS2", exact: true })).toBeVisible();
  return pluginPage;
}

test("cs2 is a plugin: on by default, says where the game config file is, the switch removes it, five pushes and no risk dialog", async ({ page }, testInfo) => {
  const pluginPage = await openPage(page);
  // 没在打：没收到数据、怎么开始
  await expect(pluginPage.getByTestId("cs2-connection")).toHaveText("没收到数据");
  await expect(pluginPage.getByTestId("cs2-idle")).toContainText("打开 CS2 进一局");
  // 配置文件：默认写上、写在哪、只发到本机的端口
  const cfg = pluginPage.getByTestId("cs2-cfg");
  await expect(cfg.getByTestId("cs2-cfg-state")).toHaveText("已写上");
  await expect(cfg.getByTestId("cs2-cfg-path")).toContainText("gamestate_integration_kotone.cfg");
  await expect(cfg).toContainText("23730");
  const toggle = cfg.getByTestId("cs2-write-cfg");
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await screenshot(page, { path: shot(testInfo, "cs2-page-idle") });
  // 关掉：删掉（只删 Kotone 写的那个）
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(cfg.getByTestId("cs2-cfg-state")).toHaveText("没写");
  await toggle.click();
  await expect(cfg.getByTestId("cs2-cfg-state")).toHaveText("已写上");
  // 没有风险说明（官方接口）
  await expect(page.getByTestId("plugin-risk-link")).toHaveCount(0);

  // 设置：插件开关 + 宿主画的五条推送
  await page.getByTestId("plugin-settings-button").click();
  await expect(page.getByTestId("cs2-settings")).toBeVisible();
  for (const id of ["bomb", "bomb-ten", "match-point", "halftime", "match-end"]) {
    await expect(page.getByTestId(`plugin-cs2-push-${id}-row`)).toBeVisible();
  }
  await expect(page.getByTestId("plugin-cs2-push-bomb-row")).toContainText("下包倒数");
  await expect(page.getByTestId("plugin-cs2-push-match-point-row")).toContainText("赛点");
  await expect(page.getByTestId("plugin-risk-link")).toHaveCount(0);
  await screenshot(page, { path: shot(testInfo, "cs2-settings") });

  // 关掉插件：页面只剩打开的按钮
  await page.getByTestId("plugin-cs2-enabled").click();
  await page.getByTestId("plugin-settings-button").click();
  await expect(pluginPage.getByTestId("plugin-cs2-off")).toBeVisible();
  await pluginPage.getByTestId("plugin-cs2-enabled").click();
  await expect(pluginPage.getByTestId("cs2-cfg")).toBeVisible();
});

test("in a match the page shows the score from my side, my stats, the rounds and the bomb counting down", async ({ page }, testInfo) => {
  await page.addInitScript(() => { (globalThis as { __kotoneCs2Match?: string }).__kotoneCs2Match = "live"; });
  const pluginPage = await openPage(page);
  await expect(pluginPage.getByTestId("cs2-connection")).toHaveText("收到数据");
  const match = pluginPage.getByTestId("cs2-this-match");
  await expect(match.getByTestId("cs2-match-map")).toHaveText("荒漠迷城 · 竞技");
  await expect(match.getByTestId("cs2-match-phase")).toHaveText("对局中 · 第 8 回合");
  // 我在 T：我们 4 : 3 对面
  await expect(match.getByTestId("cs2-match-score")).toHaveText(/我们\s*4\s*:\s*3\s*对面/);
  await expect(match.getByTestId("cs2-match-side")).toHaveText("你在 T");
  await expect(match.getByTestId("cs2-match-kda")).toHaveText("9 / 5 / 2");
  await expect(match.getByTestId("cs2-round")).toHaveCount(7);
  await expect(match.getByTestId("cs2-round").nth(1)).toHaveAttribute("data-won", "true");
  // 炸弹：下了，倒数在走
  const bomb = match.getByTestId("cs2-bomb-card");
  await expect(bomb).toHaveAttribute("data-state", "planted");
  await expect(bomb.getByTestId("cs2-bomb-state")).toHaveText("炸弹下了");
  const first = Number((await bomb.getByTestId("cs2-bomb-left").textContent())?.replace(/\D/g, ""));
  expect(first).toBeGreaterThan(20);
  expect(first).toBeLessThanOrEqual(28);
  await screenshot(page, { path: shot(testInfo, "cs2-page-live") });
  // 窄的时候（右侧栏、小窗）也排得下
  await page.setViewportSize({ width: 420, height: 820 });
  await expect(match.getByTestId("cs2-match-score")).toBeVisible();
  await screenshot(page, { path: shot(testInfo, "cs2-page-live-narrow") });
});
