import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 「插件信息」（ADR 049 D7，第 0 期）：只读的弹窗，入口在插件卡片的“⋯”菜单和插件页顶栏名字旁边的“ⓘ”。
// 只显示现在有的，没有的那一行不出现：内置插件是宿主统一填的作者 Kotone、代码仓库、MIT、Kotone 的版本，加上清单的参考；
// 第三方插件是握手时 serverInfo 的名字 / 版本 / 说明 / 主页、来源（你粘贴或导入的配置）、Kotone 清单的游戏、
// 它会运行什么（信任提示那一块），作者和许可证这一期没有。链接用系统浏览器打开（预览里是 window.open，这里记下来）。
// 浏览器预览：内置插件用 ipc/pluginsMock，第三方插件放 `window.__kotoneMcpSeed`（ipc/mcpServersMock，不连任何服务器）。

const PNG = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

const ONEDRAGON = {
  id: "onedragon",
  entry: { command: "C:\\Tools\\OneDragon\\server.exe", args: ["--mcp"] },
  status: "connected",
  trusted: true,
  title: "OneDragon Server",
  version: "2.3.1",
  description: "Runs the daily routine",
  websiteUrl: "https://github.com/example/onedragon",
  icon: PNG,
  kotoneManifest: {
    source: "server",
    manifest: { standard: "0.1", games: ["lol"], events: [{ name: "finished", label: "Run finished", data: {} }], risk: "Moves the game window" },
  },
};
/** 手改 mcp.json 加的、还没连上过：服务器什么都没说过。 */
const BARE = {
  id: "bare",
  entry: { type: "http", url: "https://bare.example.com/mcp" },
  status: "notConnected",
  trusted: false,
};
/** 连上了，但主页是 http:// 的：不当链接显示。 */
const PLAIN = {
  id: "plain",
  entry: { command: "plain-server" },
  status: "connected",
  trusted: true,
  websiteUrl: "http://insecure.example.com",
};

async function open(page: Page, seed: unknown[] = [ONEDRAGON, BARE, PLAIN]) {
  await page.addInitScript((servers) => {
    (window as unknown as { __kotoneMcpSeed: unknown }).__kotoneMcpSeed = servers;
    const opened: string[] = [];
    (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened = opened;
    window.open = ((url?: string | URL) => {
      opened.push(String(url));
      return null;
    }) as typeof window.open;
  }, seed);
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
}
const opened = (page: Page) => page.evaluate(() => (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened);

async function infoFromCard(page: Page, id: string) {
  await page.getByTestId(`plugins-card-${id}-menu`).click();
  await page.getByTestId(`plugins-card-${id}-info`).click();
  const dialog = page.getByTestId(`plugin-info-dialog-${id}`);
  await expect(dialog).toBeVisible();
  return dialog;
}

test("a built-in plugin's info: Kotone as author, its folder in the repo, MIT, Kotone's version, references, what it can do", async ({ page }, testInfo) => {
  await open(page);
  const dialog = await infoFromCard(page, "lol");
  // “⋯”菜单不进插件：还在插件页
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  await expect(dialog).toHaveAttribute("data-remote", "false");
  await expect(dialog.getByTestId("plugin-info-name")).toHaveText("英雄联盟");
  await expect(dialog.getByTestId("plugin-info-logo")).toHaveAttribute("data-logo", "file");
  await expect(dialog.getByTestId("plugin-info-version")).toHaveText(/^版本 \d+\.\d+\.\d+/);
  await expect(dialog.getByTestId("plugin-info-description")).not.toBeEmpty();
  await expect(dialog.getByTestId("plugin-info-author")).toHaveText("Kotone");
  await expect(dialog.getByTestId("plugin-info-repository")).toHaveText("github.com/l1veIn/kotone/tree/main/plugins/lol");
  await expect(dialog.getByTestId("plugin-info-license")).toHaveText("MIT");
  await expect(dialog.getByTestId("plugin-info-source")).toHaveText("Kotone 内置");
  await expect(dialog.getByTestId("plugin-info-reference")).toHaveText(["LeagueAkari", "aramhex"]);
  await expect(dialog.getByTestId("plugin-info-notices")).toHaveText("许可说明全文");
  // 内置插件没有的：主页、它会运行什么、标准版本
  await expect(dialog.getByTestId("plugin-info-homepage")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-runs")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-standard")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-id")).toHaveText("插件 id：lol");

  // 它能做什么：游戏（名字从游戏配置来）、工具 / 页面 / 事件 / 推送的数字，点开看名字
  await expect(dialog.getByTestId("plugin-info-games")).toHaveText("游戏：League of Legends");
  for (const key of ["tools", "pages", "events", "pushes"]) {
    const group = dialog.getByTestId(`plugin-info-${key}`);
    await expect(group).toBeVisible();
    expect(Number(await group.getAttribute("data-count"))).toBeGreaterThan(0);
  }
  const pushes = dialog.getByTestId("plugin-info-pushes");
  await expect(pushes.getByTestId("plugin-info-pushes-item").first()).toBeHidden();
  await pushes.locator("summary").click();
  await expect(pushes.getByTestId("plugin-info-pushes-item").first()).toBeVisible();
  await screenshot(dialog, { path: testInfo.outputPath("lol-info.png") });

  // 链接用系统浏览器打开
  await dialog.getByTestId("plugin-info-repository").click();
  await dialog.getByTestId("plugin-info-reference").first().click();
  await dialog.getByTestId("plugin-info-notices").click();
  expect(await opened(page)).toEqual([
    "https://github.com/l1veIn/kotone/tree/main/plugins/lol",
    "https://github.com/LeagueAkari/LeagueAkari",
    "https://github.com/l1veIn/kotone/blob/main/THIRD_PARTY_NOTICES.md",
  ]);

  // “风险和许可”打开现有的弹窗
  await dialog.getByTestId("plugin-info-risk-link").click();
  await expect(page.getByTestId("plugin-risk-dialog-lol")).toBeVisible();
  await expect(dialog).toBeHidden();
});

test("a built-in plugin without references or risks has no such rows; the plugin page bar opens the same info", async ({ page }) => {
  await open(page);
  await page.getByTestId("plugins-card-interpreter-open").click();
  await expect(page.locator('[data-testid="plugin-page"][data-plugin-id="interpreter"]')).toBeVisible();
  await page.getByTestId("plugin-info-button").click();
  const dialog = page.getByTestId("plugin-info-dialog-interpreter");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("plugin-info-author")).toHaveText("Kotone");
  await expect(dialog.getByTestId("plugin-info-repository")).toHaveText("github.com/l1veIn/kotone/tree/main/plugins/interpreter");
  await expect(dialog.getByTestId("plugin-info-references")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-games")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-risk-link")).toHaveCount(0);
  await dialog.getByTestId("plugin-info-close").click();
  await expect(dialog).toBeHidden();
});

test("a third-party plugin's info: what the server said, its homepage, where it came from, what it runs; no author or license", async ({ page }, testInfo) => {
  await open(page);
  const dialog = await infoFromCard(page, "onedragon");
  await expect(dialog).toHaveAttribute("data-remote", "true");
  await expect(dialog.getByTestId("plugin-info-name")).toHaveText("onedragon");
  await expect(dialog.getByTestId("plugin-info-subtitle")).toHaveText("OneDragon Server");
  await expect(dialog.getByTestId("plugin-info-logo")).toHaveAttribute("data-logo", "file");
  await expect(dialog.getByTestId("plugin-info-version")).toHaveText("版本 2.3.1");
  await expect(dialog.getByTestId("plugin-info-description")).toHaveText("Runs the daily routine");
  await expect(dialog.getByTestId("plugin-info-homepage")).toHaveText("github.com/example/onedragon");
  await expect(dialog.getByTestId("plugin-info-source")).toHaveText("你粘贴或导入的配置");
  // 这一期第三方插件没有作者、许可证、代码仓库、参考
  for (const row of ["author", "license", "repository", "references"]) {
    await expect(dialog.getByTestId(`plugin-info-${row}`)).toHaveCount(0);
  }
  await expect(dialog.getByTestId("plugin-info-games")).toHaveText("游戏：League of Legends");
  await expect(dialog.getByTestId("plugin-info-tools")).toHaveAttribute("data-count", "2");
  await expect(dialog.getByTestId("plugin-info-events")).toHaveAttribute("data-count", "1");
  await expect(dialog.getByTestId("plugin-info-pages")).toHaveCount(0);
  await expect(dialog.getByTestId("plugin-info-pushes")).toHaveCount(0);
  await dialog.getByTestId("plugin-info-tools").locator("summary").click();
  await expect(dialog.getByTestId("plugin-info-tools-item")).toHaveText(["Echo", "Add"]);
  // 它会运行什么：信任提示里那一块
  await expect(dialog.getByTestId("plugin-info-facts-runs-command")).toHaveText("C:\\Tools\\OneDragon\\server.exe --mcp");
  await expect(dialog.getByTestId("plugin-info-id")).toHaveText("插件 id：onedragon");
  await expect(dialog.getByTestId("plugin-info-standard")).toHaveText("插件标准：0.1");
  await screenshot(dialog, { path: testInfo.outputPath("third-party-info.png") });

  await dialog.getByTestId("plugin-info-homepage").click();
  expect(await opened(page)).toEqual(["https://github.com/example/onedragon"]);
  await dialog.getByTestId("plugin-info-risk-link").click();
  const risk = page.getByTestId("plugin-risk-dialog-onedragon");
  await expect(risk).toBeVisible();
  await expect(risk.getByTestId("plugin-risk-onedragon-declared")).toContainText("Moves the game window");
});

test("a third-party plugin that never connected shows only what Kotone knows; an http:// homepage is not a link", async ({ page }) => {
  await open(page);
  const dialog = await infoFromCard(page, "bare");
  await expect(dialog.getByTestId("plugin-info-logo")).toHaveAttribute("data-logo", "fallback");
  for (const row of ["version", "subtitle", "homepage", "author", "license", "repository", "references", "abilities", "standard"]) {
    await expect(dialog.getByTestId(`plugin-info-${row}`)).toHaveCount(0);
  }
  await expect(dialog.getByTestId("plugin-info-source")).toHaveText("你粘贴或导入的配置");
  await expect(dialog.getByTestId("plugin-info-facts-runs-url")).toHaveText("https://bare.example.com/mcp");
  // 第三方插件总有“风险和许可”（它是别人写的程序）
  await expect(dialog.getByTestId("plugin-info-risk-link")).toBeVisible();
  await dialog.getByTestId("plugin-info-close").click();

  const plain = await infoFromCard(page, "plain");
  await expect(plain.getByTestId("plugin-info-version")).toHaveText("版本 1.0.0");
  await expect(plain.getByTestId("plugin-info-homepage")).toHaveCount(0);
  await expect(plain.getByTestId("plugin-info-standard")).toHaveCount(0);
});
