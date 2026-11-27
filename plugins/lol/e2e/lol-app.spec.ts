// 英雄联盟的 MCP App（ADR 039 D5；MCP 方案第 3 期）在 Kotone 里的样子：插件页整页是插件给的独立 HTML（从“主页”开始，
// 导航在 App 里），宿主放进沙盒 iframe（McpAppFrame + ext-apps 的 AppBridge）。浏览器预览里沙盒代理页和 App 由 Vite 开发服务器旁边的小服务照回环代理
// 的路径、在 127.0.0.1 的另一个端口上给（apps/desktop/vite.config.ts 的 mcpAppsDevServer），App 是 plugins/lol/app/dist
// （test:e2e 先打包）。
// 只看宿主这一侧：握手完成、插件页是 fullscreen、聊天卡片是 inline 并且用的是推过去的结果、App 调工具时宿主绑定的是
// 英雄联盟插件、调别的插件的工具被拒、App 请宿主打开本插件的页面。App 自己长什么样由 App 的测试管（之后整个重做）。
import { expect, test, type Frame, type Page, type TestInfo } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

const TIER = "ui://kotone/lol/tier";
const HOME = "ui://kotone/lol/home";
const MATCH = "ui://kotone/lol/match";

/** 插件的 App（ADR 038）：左侧栏「插件」→ 点它的卡片（和 apps/desktop/e2e/nav.ts 的 openPlugin 一样）。 */
async function openPlugin(page: Page, id: string) {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId(`plugins-card-${id}-open`).click();
  await expect(page.locator(`[data-testid="plugin-page"][data-plugin-id="${id}"]`)).toBeVisible();
}

/** 截图放在仓库的 target/plugin-shots/round7/；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round7", `${name}.png`);

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

async function lolPage(page: Page) {
  await openPlugin(page, "lol");
  return page.getByTestId("plugin-page-lol");
}

/** App 所在的那一层 iframe（沙盒代理页里面那一层，地址是 /<密钥>/app/lol/<页面>）。 */
async function appFrame(page: Page, which: string): Promise<Frame> {
  const pattern = new RegExp(`/app/lol/(${which})$`);
  await expect.poll(() => page.frames().filter((f) => pattern.test(f.url())).length, { timeout: 15_000 }).toBeGreaterThan(0);
  return page.frames().find((f) => pattern.test(f.url()))!;
}

/** 预览里插件工具被调过的记录（pluginsMock 记的）。 */
const calls = (page: Page) => page.evaluate(() => (window as unknown as { __kotonePluginCalls: { plugin: string; tool: string }[] }).__kotonePluginCalls ?? []);

/** 在 App 那一层发一条 JSON-RPC 请求给宿主（经沙盒代理页转），等回来的那一条。 */
function rpc(frame: Frame, id: number, method: string, params: unknown): Promise<{ result?: Record<string, unknown>; error?: { message: string } }> {
  return frame.evaluate(([id, method, params]) => new Promise((resolve) => {
    const listener = (event: MessageEvent) => {
      if (event.data?.id === id) {
        window.removeEventListener("message", listener);
        resolve(event.data);
      }
    };
    window.addEventListener("message", listener);
    window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
  }), [id, method, params] as const);
}

test("the plugin page is the plugin's MCP App: a sandboxed iframe on another origin that fetches through the lol plugin", async ({ page }, testInfo) => {
  await open(page);
  const pluginPage = await lolPage(page);
  const frame = pluginPage.locator(`[data-testid="mcp-app-frame"][data-uri="${HOME}"]`);
  await expect(frame).toHaveAttribute("data-display", "fullscreen");
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect(frame).toHaveAttribute("data-plugin", "lol");
  const iframe = frame.getByTestId("mcp-app-iframe");
  await expect(iframe).toHaveAttribute("sandbox", "allow-scripts allow-same-origin allow-forms");
  // 沙盒代理页在另一个源上（127.0.0.1 的另一个端口），路径照回环代理
  await expect(iframe).toHaveAttribute("src", /^http:\/\/127\.0\.0\.1:\d+\/[0-9a-f]+\/sandbox\/lol\/home$/);
  expect(new URL((await iframe.getAttribute("src"))!).port).not.toBe(new URL(page.url()).port);
  // 插件页直接打开时，App 向英雄联盟插件查询当前召唤师。
  await expect.poll(async () => (await calls(page)).filter((c) => c.tool === "lol.player")).toContainEqual({ plugin: "lol", tool: "lol.player" });
  const app = await appFrame(page, "home");
  // App 里换到强度榜：App 自己调 kotone_lol_tier
  await app.getByTestId("lol-app-tab-tier").click();
  await expect.poll(async () => (await calls(page)).filter((c) => c.tool === "lol.tier")).toEqual([{ plugin: "lol", tool: "lol.tier" }]);
  await expect(app.locator("body")).toContainText("腾讯 101");
  await expect(app.locator("body")).toContainText("T0");
  await screenshot(page, { path: shot(testInfo, "tier-app-plugin-page") });
});

test("the App only reaches its own plugin: lol tools work, other plugins' tools are refused, its settings link opens the plugin's settings", async ({ page }) => {
  await open(page);
  const pluginPage = await lolPage(page);
  await expect(pluginPage.locator(`[data-testid="mcp-app-frame"][data-uri="${HOME}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "home");
  await expect(app.getByTestId("lol-home-name")).toContainText("玩家甲");
  // 切到静态榜单，采样期间避免主页定时查询对局状态。
  await app.getByTestId("lol-app-tab-tier").click();
  await expect(app.getByTestId("lol-app-tier-row")).toHaveCount(12);
  // 从这里起宿主替 App 调的工具（别的界面，例如左侧栏的播放条，也会调自己插件的工具）
  const before = (await calls(page)).length;
  const since = async () => (await calls(page)).slice(before);

  // 只给页面用的 lol 工具：认 MCP 名，结果照 MCP 的 CallToolResult
  const champions = await rpc(app, 90_001, "tools/call", { name: "kotone_lol_champions", arguments: {} });
  expect(champions.error).toBeUndefined();
  expect(Array.isArray((champions.result?.structuredContent as { champions?: unknown[] })?.champions)).toBe(true);
  expect((champions.result?.content as { type: string }[])[0].type).toBe("text");
  // 别的插件的工具：MCP 名、Kotone 名都不行；页面说自己是谁也没用
  for (const [id, name] of [[90_002, "kotone_music_play"], [90_003, "music.play"], [90_004, "kotone_game_chat_send"]] as const) {
    const refused = await rpc(app, id, "tools/call", { name, arguments: { plugin: "music" } });
    expect(refused.result, name).toBeUndefined();
    expect(refused.error?.message, name).toContain("cannot call");
  }
  // 读资源只读自己插件的
  const own = await rpc(app, 90_005, "resources/read", { uri: MATCH });
  expect((own.result?.contents as { mimeType: string }[])[0].mimeType).toBe("text/html;profile=mcp-app");
  const other = await rpc(app, 90_006, "resources/read", { uri: "ui://kotone/music/home" });
  expect(other.error).toBeDefined();
  // 宿主只替英雄联盟插件调过工具，被拒的一个都没调
  expect(await since()).toEqual([{ plugin: "lol", tool: "lol.champions" }]);

  // 别的插件的 ui:// 地址、不是网址的：不打开
  for (const [id, url] of [[90_007, "ui://kotone/music/home"], [90_008, "javascript:alert(1)"]] as const) {
    expect((await rpc(app, id, "ui/open-link", { url })).result?.isError, url).toBe(true);
  }
  // App 请宿主打开本插件的设置（App 里的“去设置”）：插件页切到设置，再点“设置”回到 App
  await app.evaluate(() => window.parent.postMessage({ jsonrpc: "2.0", id: 90_009, method: "ui/open-link",
    params: { url: "ui://kotone/lol/settings" } }, "*"));
  await expect(page.getByTestId("plugin-page")).toHaveAttribute("data-view", "settings");
  await expect(page.getByTestId("plugin-settings").getByTestId("plugin-lol-source-client")).toBeVisible();
  await page.getByTestId("plugin-settings-button").click();
  await expect(page.getByTestId("plugin-page")).toHaveAttribute("data-view", "app");
});

test("the tier card in the chat is the same App, inline, shown with the result she got (it does not fetch again)", async ({ page }, testInfo) => {
  await open(page);
  const card = page.locator(`[data-testid="plugin-card"][data-uri="${TIER}"]`);
  await card.scrollIntoViewIfNeeded();
  const frame = card.locator('[data-testid="mcp-app-frame"]');
  await expect(frame).toHaveAttribute("data-display", "inline");
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "tier");
  await expect(app.locator("body")).toContainText("腾讯 101");
  // 卡片用的是推过去的结果：宿主没替它再调 lol.tier
  expect((await calls(page)).filter((c) => c.tool === "lol.tier")).toEqual([]);
  // 高度跟着 App 报的走（在卡片的范围里）
  const box = await frame.boundingBox();
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(120);
  expect(box?.height ?? 0).toBeLessThanOrEqual(520);
  await screenshot(card, { path: shot(testInfo, "tier-app-chat-card") });

  // 点卡片头上一行：右侧栏打开完整页面（fullscreen 的同一个 App）
  await card.getByTestId("plugin-card-open").click();
  await expect(page.locator(`[data-testid="mcp-app-frame"][data-uri="${TIER}"][data-display="fullscreen"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
});

test("both CSPs of the main window let the App's iframe in (frame-src http://127.0.0.1:*)", () => {
  const app = join(__dirname, "..", "..", "..", "apps", "desktop");
  const meta = /http-equiv="Content-Security-Policy"\s+content="([^"]+)"/.exec(readFileSync(join(app, "index.html"), "utf8"))?.[1] ?? "";
  const conf = JSON.parse(readFileSync(join(app, "src-tauri", "tauri.conf.json"), "utf8")).app.security.csp as string;
  for (const policy of [meta, conf]) expect(policy).toContain("frame-src http://127.0.0.1:*");
});
