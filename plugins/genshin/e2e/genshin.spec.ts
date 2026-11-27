// 原神插件（ADR 047 第 3 期，驱动 BetterGI）：左侧栏「插件」里有它的卡片（默认关）；打开时弹一次“风险和许可”（米哈游的用户协议、
// BetterGI 的 GPL-3.0）；页面上是 BetterGI 在哪、它的通知 Webhook 有没有指向 Kotone（一键填好）、能跑的一条龙配置 / 配置组 / 计划
// 和“跑”按钮（受原神游戏配置的“允许自动化输入”管；BetterGI 开着时只能交计划）、最近的运行、怎么停（没有停止接口，如实说）。
// 浏览器预览的后端是 plugins/genshin/ui/mock.ts（假的 BetterGI；`window.__kotoneGenshin` 切开着 / 不允许自动化输入 / 没找到）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

/** 截图放在仓库的 target/plugin-shots/genshin/；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "genshin", `${name}.png`);

type Knobs = { running?: boolean; automation?: boolean; found?: boolean };

/** 打开插件页（插件先打开：清单默认关）。预览里只有英雄联盟的风险说明当作看过了：打开原神时弹一次，`keepDialog` 时留着给测试看。 */
async function openPage(page: Page, knobs: Knobs = {}, keepDialog = false) {
  await page.addInitScript((k) => { (globalThis as { __kotoneGenshin?: Knobs }).__kotoneGenshin = k; }, knobs);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("settings-nav-plugins").click();
  const card = page.getByTestId("plugins-card-genshin");
  await expect(card).toContainText("原神");
  await expect(card.getByTestId("plugins-card-genshin-enabled")).toHaveAttribute("aria-checked", "false");
  await card.getByTestId("plugins-card-genshin-open").click();
  const pluginPage = page.getByTestId("plugin-page-genshin");
  await expect(pluginPage.getByTestId("plugin-genshin-off")).toBeVisible();
  await pluginPage.getByTestId("plugin-genshin-enabled").click();
  const dialog = page.getByTestId("plugin-risk-dialog-genshin");
  await expect(dialog).toBeVisible();
  if (!keepDialog) {
    await dialog.getByTestId("plugin-risk-ok").click();
    await expect(dialog).toHaveCount(0);
  }
  return pluginPage;
}

test("genshin is off by default; turning it on shows the risks once, then where BetterGI is, the webhook and what it can run", async ({ page }, testInfo) => {
  const pluginPage = await openPage(page, {}, true);
  // 风险和许可：一个弹窗，风险（米哈游的用户协议、BetterGI 自己的说明）在前，GPL-3.0 在后
  const dialog = page.getByTestId("plugin-risk-dialog-genshin");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("plugin-risk-genshin-bettergi")).toContainText("米哈游的用户协议禁止使用未经许可的第三方工具");
  await expect(dialog.getByTestId("plugin-notice-genshin-bettergi")).toContainText("GPL-3.0");
  await screenshot(page, { path: shot(testInfo, "genshin-risk") });
  await dialog.getByTestId("plugin-risk-ok").click();
  await expect(dialog).toHaveCount(0);

  // BetterGI 在哪、版本、开没开
  await expect(pluginPage.getByTestId("genshin-install")).toContainText("D:\\BetterGI");
  await expect(pluginPage.getByTestId("genshin-install")).toContainText("版本 0.67.0");
  await expect(pluginPage.getByTestId("genshin-running")).toHaveText("BetterGI 没开");

  // 通知 Webhook 没指向 Kotone：地址 + 一键填好
  const webhook = pluginPage.getByTestId("genshin-webhook");
  await expect(webhook).toHaveAttribute("data-to-kotone", "false");
  await expect(webhook).toContainText("没指向 Kotone");
  await expect(webhook.getByTestId("genshin-webhook-url")).toContainText("http://127.0.0.1:38648/genshin/");
  await webhook.getByTestId("genshin-webhook-fill").click();
  await expect(page.getByText("填好了，原来的配置备份在")).toBeVisible();
  await expect(webhook).toHaveAttribute("data-to-kotone", "true");
  await expect(webhook).toContainText("跑完、出错会推到聊天里");
  await expect(webhook.getByTestId("genshin-webhook-fill")).toHaveCount(0);

  // 能跑的：一条龙（选中的那个标出来）、配置组、计划
  const runnable = pluginPage.getByTestId("genshin-runnable");
  await expect(runnable.getByTestId("genshin-dragon").first()).toContainText("默认配置");
  await expect(runnable.getByTestId("genshin-dragon").first()).toContainText("选中的");
  await expect(runnable.getByTestId("genshin-group")).toHaveCount(2);
  await expect(runnable.getByTestId("genshin-plan")).toContainText("早上的日常");
  await screenshot(page, { path: shot(testInfo, "genshin-page") });

  // 跑一条龙：提示已启动，最近的运行里多一条“正在启动”
  await runnable.getByTestId("genshin-run-dragon").first().click();
  await expect(page.getByText("已启动 BetterGI：默认配置")).toBeVisible();
  const recent = pluginPage.getByTestId("genshin-recent");
  await expect(recent.getByTestId("genshin-run").first()).toHaveAttribute("data-status", "starting");
  await expect(recent.getByTestId("genshin-run").nth(1)).toHaveAttribute("data-status", "succeeded");

  // 怎么停：如实说没有停止接口
  await expect(pluginPage.getByTestId("genshin-stop")).toContainText("BetterGI 没有给外部停止的接口");
  await expect(pluginPage.getByTestId("genshin-stop")).toContainText("不会结束它的进程");

  // 设置：插件开关、驱动 BetterGI、BetterGI 在哪；风险在“风险和许可”里
  await page.getByTestId("plugin-settings-button").click();
  await expect(page.getByTestId("genshin-settings")).toBeVisible();
  await expect(page.getByTestId("genshin-path")).toHaveAttribute("placeholder", "自己找");
  await expect(page.getByTestId("plugin-risk-link")).toBeVisible();
  await expect(page.getByTestId("plugin-genshin-push-runs-row")).toContainText("BetterGI 的运行");
});

test("while BetterGI is open only plans can be started and the webhook can't be filled", async ({ page }) => {
  const pluginPage = await openPage(page, { running: true });
  await expect(pluginPage.getByTestId("genshin-running")).toHaveText("BetterGI 开着");
  await expect(pluginPage.getByTestId("genshin-busy-note")).toContainText("只能交给它计划");
  await expect(pluginPage.getByTestId("genshin-run-dragon").first()).toBeDisabled();
  await expect(pluginPage.getByTestId("genshin-run-group").first()).toBeDisabled();
  await expect(pluginPage.getByTestId("genshin-run-plan")).toBeEnabled();
  await expect(pluginPage.getByTestId("genshin-webhook-fill")).toBeDisabled();
  await expect(pluginPage.getByTestId("genshin-webhook")).toContainText("BetterGI 开着时不能改它的配置");
  await pluginPage.getByTestId("genshin-run-plan").click();
  await expect(page.getByText("已启动 BetterGI：早上的日常")).toBeVisible();
});

test("no automation input for Genshin: the run buttons are off and the page says where to turn it on", async ({ page }) => {
  const pluginPage = await openPage(page, { automation: false });
  await expect(pluginPage.getByTestId("genshin-automation-off")).toContainText("允许自动化输入");
  for (const id of ["genshin-run-dragon", "genshin-run-group", "genshin-run-plan"]) {
    await expect(pluginPage.getByTestId(id).first()).toBeDisabled();
  }
});

test("BetterGI not found: the page says where it looked and links to the setting", async ({ page }) => {
  const pluginPage = await openPage(page, { found: false });
  const missing = pluginPage.getByTestId("genshin-not-found");
  await expect(missing).toContainText("没找到 BetterGI");
  await expect(missing.getByTestId("genshin-searched")).toContainText("D:\\BetterGI");
  await missing.getByTestId("genshin-open-settings").click();
  await expect(page.getByTestId("genshin-settings")).toBeVisible();
});
