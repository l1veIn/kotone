// 同传插件（ADR 021、038）：左侧栏「插件」里有它的卡片（默认开），点开它的页面：设好听哪个程序、译成什么、怎么显示
// （写进 config.json `plugins.interpreter.settings`），点“开始”就替你对她说那句话、她的耳朵开始听；“停止”就不听了。
// 关掉插件时页面只剩开关和一句说明。浏览器预览里她的耳朵是模拟的（ipc/interpreterMock.ts）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";

/** 截图放在仓库的 target/plugin-shots/step4/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "step4", `${name}.png`);

/** 页面里的设置（字符串形式求值：这个目录的测试按 CommonJS 编译，函数里的 import() 会被改写）。 */
async function settings(page: Page): Promise<Record<string, unknown> & {
  plugins?: Record<string, { enabled?: boolean; settings?: Record<string, unknown> }>;
}> {
  return page.evaluate("import('/src/lib/ipc.ts').then((ipc) => ipc.getSettings())");
}

test("interpreter is a plugin: on by default, set the program, language and display, start and stop interpreting", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // 左侧栏「插件」：同传的卡片，默认开
  await page.getByTestId("settings-nav-plugins").click();
  const card = page.getByTestId("plugins-card-interpreter");
  await expect(card).toContainText("同传");
  await expect(card.getByTestId("plugins-card-interpreter-enabled")).toHaveAttribute("aria-checked", "true");

  // 点卡片进插件的 App：还没在听
  await card.getByTestId("plugins-card-interpreter-open").click();
  const pluginPage = page.getByTestId("plugin-page-interpreter");
  await expect(pluginPage.getByRole("heading", { name: "同传", exact: true })).toBeVisible();
  await expect(pluginPage.getByTestId("plugin-interpreter-off")).toHaveCount(0);
  const listening = pluginPage.getByTestId("interpreter-listening");
  await expect(listening).toHaveText("没在听");
  await expect(pluginPage.getByTestId("interpreter-app")).toHaveAttribute("placeholder", "现在的游戏");
  await expect(pluginPage.getByTestId("interpreter-target")).toContainText("中文");
  await expect(pluginPage.getByTestId("interpreter-display")).toContainText("弹幕");
  await expect(pluginPage.getByTestId("interpreter-note")).toContainText("停止同传");

  // 听 Discord、译成日语、用气泡：写进插件自己的设置
  await pluginPage.getByTestId("interpreter-app").fill("Discord.exe");
  await pluginPage.getByTestId("interpreter-app").press("Enter");
  await pluginPage.getByTestId("interpreter-target").click();
  await page.getByRole("option", { name: "日语" }).click();
  await expect(pluginPage.getByTestId("interpreter-target")).toContainText("日语");
  await pluginPage.getByTestId("interpreter-display").click();
  await page.getByRole("option", { name: "气泡" }).click();
  await expect(pluginPage.getByTestId("interpreter-display")).toContainText("气泡");
  await expect.poll(async () => (await settings(page)).plugins?.interpreter?.settings).toEqual({
    app: "Discord.exe",
    target: "日语",
    display: "bubble",
  });

  // 开始：替你对她说那句话，她的耳朵在听 Discord
  await pluginPage.getByTestId("interpreter-start").click();
  await expect(page.getByText("帮我盯着队友说的话，当同传：听 Discord.exe，每句外语翻成日语，用气泡显示。")).toBeVisible();
  await expect(listening).toHaveText("正在听：Discord.exe");
  await page.screenshot({ path: shot(testInfo, "interpreter-page") });

  // 停止：对她说“停止同传”，不听了
  await pluginPage.getByTestId("interpreter-stop").click();
  await expect(page.getByText("已替你对她说：停止同传")).toBeVisible();
  await expect(listening).toHaveText("没在听");

  // 关掉插件：只剩开关和一句说明；设置还在
  await pluginPage.getByTestId("plugin-interpreter-enabled").click();
  await expect(pluginPage.getByTestId("plugin-interpreter-enabled")).toHaveAttribute("aria-checked", "false");
  await expect(pluginPage.getByTestId("plugin-interpreter-off")).toBeVisible();
  await expect(pluginPage.getByTestId("interpreter-settings")).toHaveCount(0);
  await expect(pluginPage.getByTestId("interpreter-start")).toHaveCount(0);
  await expect.poll(async () => (await settings(page)).plugins?.interpreter).toMatchObject({
    enabled: false,
    settings: { app: "Discord.exe", target: "日语", display: "bubble" },
  });
});

test("empty program means the current game, and her ui.open lands on the interpreter page in the right sidebar", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  // 旧对话里的“去看看：设置 → 插件 → 同传”
  await page.evaluate("import('/src/lib/stores/navigation.ts').then((n) => n.openLocation('设置 → 插件 → 同传'))");
  const pluginPage = page.getByTestId("plugin-page-interpreter");
  await expect(pluginPage).toBeVisible();
  await pluginPage.getByTestId("interpreter-start").click();
  await expect(page.getByText("帮我盯着队友说的话，当同传：听 现在的游戏，每句外语翻成中文，用弹幕显示。")).toBeVisible();
  await expect(pluginPage.getByTestId("interpreter-listening")).toHaveText("正在听：TslGame.exe");

  // 她打开插件的页面：玩家正在插件页，不切过去（共享资源评估第二节第 6 条）；点提示上的“打开”在聊天页的右侧栏打开
  expect(await page.evaluate("window.kotoneUiOpen({ uri: 'ui://kotone/interpreter/home' })")).toBe("waiting");
  await expect(pluginPage).toBeVisible();
  await page.locator("[data-sonner-toast]").filter({ hasText: "同传" }).getByRole("button", { name: "打开" }).click();
  const view = page.getByTestId("plugin-view-interpreter");
  await expect(view.getByTestId("plugin-interpreter-enabled")).toBeVisible();
  await expect(view.getByTestId("interpreter-listening")).toHaveText("正在听：TslGame.exe");
});
