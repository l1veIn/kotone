// 结算和房间里的两条推送（ADR 043 B 轮第 3 条；讨论记录第二节推送清单的“排队 / 房间”和“结算”两行）：
// - 这局总结（推送 `summary`，页面 `ui://kotone/lol/game` 的卡片）：海斗、斗魂竞技场再加一块“海克斯”，先显示对面每个人的
//   （一人一行，图标 + 名字），点“我方”换成我方的；没有海克斯的模式没有这一块。单局详情全屏每队下面也有这一块。
// - 今天的状态（推送 `today`，页面 `ui://kotone/lol/today`）：进房间或排队时推来，一天一张；打完一局回到房间时同一张原地
//   更新（App 收到新的数据重画，不重新加载）。窄的时候（悬浮窗的小窗约 360 px）也放得下。
// 用 @tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件，插件的命令交给浏览器预览的 pluginsMock，卡片里是真的 App
// （vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。数据是 lolMock.ts 的 sampleSummary / sampleToday。
import { expect, test, type Frame, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

// 测试页是 Playwright 现给的（route.fulfill），Chrome 按“本地网络访问”的规矩会挡掉它去连回环地址上的 MCP App 代理；
// 这个文件里关掉这项检查（真的 Kotone 窗口里页面本来就在本机）。和 push.spec.ts 一样。
test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const SUMMARY_KEY = "summary:9000001";
const TODAY_KEY = "today:2026-11-26";
const GAME = "ui://kotone/lol/game";
const TODAY = "ui://kotone/lol/today";

/** 截图放在仓库的 target/plugin-shots/feed-and-orb-b/（实施记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "feed-and-orb-b", `${name}.png`);

interface Bridge {
  push(event: unknown): Promise<void>;
  /** 今天的状态那条 Notice：今天打了几局（5 局；6 = 又赢了一局）。 */
  today(games?: number): unknown;
}
declare global {
  interface Window {
    lolPostTest: Bridge;
  }
}

/** 一段对话：一条“在吗 / 在的”，后面是推来的一张卡片：这局总结（`rift` 时去掉海克斯，像峡谷那样）或今天的状态。 */
async function open(page: Page, which: "summary" | "today", options: { rift?: boolean } = {}) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__lol-post-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__lol-post-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, SUMMARY_KEY, TODAY_KEY, GAME, TODAY, which, options }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    const { sampleSummary, sampleToday } = await import("/src/lib/ipc/lolMock.ts");
    // i18n-ignore-start：示例对话
    const summaryData = sampleSummary();
    if (options.rift) for (const t of summaryData.teams) for (const p of t.players) p.augments = [];
    const summary = {
      role: "notice", text: "英雄联盟 推送：这局总结", atMs: T - 30_000,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "summary", title: "这局总结", key: SUMMARY_KEY,
        card: { title: "这局总结", ui: { uri: GAME, params: { gameId: summaryData.gameId } }, structuredContent: summaryData },
      },
    };
    const today = (games = 5) => ({
      role: "notice", text: "英雄联盟 推送：今天的状态", atMs: T - 20_000,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "today", title: "今天的状态", key: TODAY_KEY,
        card: { title: "今天的状态", ui: { uri: TODAY }, structuredContent: sampleToday(games) },
        ...(games === 5 ? {} : { updatedAtMs: T }),
      },
    });
    const conversation = {
      version: 1, id: "c-1", title: "这局", createdAtMs: T - 86_400_000, updatedAtMs: T - 20_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        which === "summary" ? summary : today(),
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c-1", running: null, notReadyReason: null,
      conversations: [{ id: "c-1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    Object.assign(window, { lolPostTest: { async push(event: unknown) { await emit("kotone://push", event); }, today } });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversation);
        case "agent_held_cards": return [];
        case "automation_runs": return { runs: [] };
        case "usage_conversation": return null;
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
        case "plugins_list": return plugins.listPlugins();
        case "plugin_app": return plugins.pluginApp((a as { uri: string }).uri);
        case "plugin_resource": return plugins.pluginResource((a as { plugin: string }).plugin, (a as { uri: string }).uri);
        case "plugin_call": return plugins.pluginCall((a as { plugin: string }).plugin, (a as { tool: string }).tool, (a as { arguments: Record<string, unknown> }).arguments);
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, SUMMARY_KEY, TODAY_KEY, GAME, TODAY, which, options });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);

/** 卡片里那一层 App（沙盒代理页里面那一层，地址是 /<密钥>/app/lol/<页面>）。 */
async function appFrame(page: Page, which: string): Promise<Frame> {
  const pattern = new RegExp(`/app/lol/${which}$`);
  await expect.poll(() => page.frames().filter((f) => pattern.test(f.url())).length, { timeout: 15_000 }).toBe(1);
  return page.frames().find((f) => pattern.test(f.url()))!;
}

/** App 那一层窄到 `width`（悬浮窗的小窗约 360 px）：根元素不横着溢出。 */
async function narrow(app: Frame, testId: string, width = 360) {
  await app.evaluate((w) => { document.body.style.width = `${w}px`; }, width);
  const root = app.getByTestId(testId);
  expect(await root.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
}

test("the match summary card lists each enemy's augments (icon + name), and 我方 switches to my team", async ({ page }, testInfo) => {
  await open(page, "summary");
  const card = pushAt(page, SUMMARY_KEY);
  await expect(card.getByTestId("push-title")).toHaveText("这局总结");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(`[data-testid="mcp-app-frame"][data-uri="${GAME}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "game");
  await expect(app.getByTestId("lol-app-game")).toHaveAttribute("data-just-ended", "true");
  const block = app.getByTestId("lol-card-augments");
  await expect(block).toHaveAttribute("data-side", "enemy");
  await expect(block).toContainText("海克斯");
  // 对面 5 个人，一人一行，每人 4 个海克斯（图标 + 名字）
  const rows = block.getByTestId("lol-game-augment-row");
  await expect(rows).toHaveCount(5);
  for (let i = 0; i < 5; i++) await expect(rows.nth(i).getByTestId("lol-game-augment")).toHaveCount(4);
  await expect(rows.first()).toHaveAttribute("data-champion", "157");
  // 按选的顺序（示例数据：亚索选的第一个是金色的“鼓声暴起 低音轰鸣”）
  await expect(rows.first().getByTestId("lol-game-augment").first().locator(".an")).toHaveText("鼓声暴起 低音轰鸣");
  await expect(rows.first().getByTestId("lol-game-augment").first()).toHaveAttribute("data-rarity", "gold");
  await expect(block).toContainText("巨像的勇气");
  await expect(block.locator(".me")).toHaveCount(0);
  await screenshot(page, { path: shot(testInfo, "summary-card-enemy-augments") });

  // 换成我方：我那一行淡蓝底
  const sides = block.getByTestId("lol-card-augments-side");
  await expect(sides).toHaveText(["对面", "我方"]);
  await expect(sides.first()).toHaveAttribute("aria-pressed", "true");
  await sides.nth(1).click();
  await expect(block).toHaveAttribute("data-side", "ally");
  await expect(block.getByTestId("lol-game-augment-row")).toHaveCount(5);
  await expect(block.getByTestId("lol-game-augment-row").first()).toHaveClass(/\bme\b/);
  await expect(block.getByTestId("lol-game-augment-row").first()).toHaveAttribute("data-champion", "39");

  // 悬浮窗的小窗（约 360 px）：不横着溢出，一行两格（第三格在第一格下面），名字放不下就省略
  await sides.first().click();
  await narrow(app, "lol-app-game");
  const chips = block.getByTestId("lol-game-augment-row").first().getByTestId("lol-game-augment");
  // 宿主可能还在按卡片高度滚动；在 iframe 内的同一次布局中比较三个格子。
  const [firstY, secondY, thirdY] = await chips.evaluateAll((cells) => cells.map((el) => el.getBoundingClientRect().top));
  expect(Math.abs(firstY - secondY)).toBeLessThan(2);
  expect(thirdY).toBeGreaterThan(firstY + 10);
  for (let i = 0; i < 4; i++) {
    expect(await chips.nth(i).evaluate((el) => el.getBoundingClientRect().right <= el.closest("[data-testid='lol-game-augment-row']")!.getBoundingClientRect().right + 1)).toBe(true);
  }
  await screenshot(page, { path: shot(testInfo, "summary-card-augments-360") });
});

test("a summary without augments (the Rift) has no augment block", async ({ page }) => {
  await open(page, "summary", { rift: true });
  const card = pushAt(page, SUMMARY_KEY);
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "game");
  await expect(app.getByTestId("lol-card-me")).toBeVisible();
  await expect(app.getByTestId("lol-card-augments")).toHaveCount(0);
  await expect(app.getByTestId("lol-game-augments")).toHaveCount(0);
});

test("the today card states today's games, the streak and the champions that lost twice; a new game updates it in place", async ({ page }, testInfo) => {
  await open(page, "today");
  const card = pushAt(page, TODAY_KEY);
  await expect(card.getByTestId("push-from")).toContainText("英雄联盟 推送");
  await expect(card.getByTestId("push-title")).toHaveText("今天的状态");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(`[data-testid="mcp-app-frame"][data-uri="${TODAY}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "today");
  const root = app.getByTestId("lol-app-today");
  await expect(root).toHaveAttribute("data-date", "2026-11-26");
  await expect(root).toContainText("11 月 26 日");
  await expect(app.getByTestId("lol-today-head")).toContainText("今天 5 局 · 2 胜 3 负");
  await expect(app.getByTestId("lol-today-head")).toContainText("胜率 40%");
  const streak = app.getByTestId("lol-today-streak");
  await expect(streak).toHaveText("现在 2 连败");
  await expect(streak).toHaveAttribute("data-streak", "-2");
  await expect(app.getByTestId("lol-today-recent").locator(".rg")).toHaveCount(5);
  // 今天输了 2 局以上：亚索 3 局 1 胜 2 负（只写事实，不评价人）
  await expect(root).toContainText("今天输了 2 局以上");
  const lost = app.getByTestId("lol-today-lost");
  await expect(lost).toHaveCount(1);
  await expect(lost).toHaveAttribute("data-champion", "157");
  await expect(lost).toContainText("亚索");
  await expect(lost).toContainText("3 局 1 胜 2 负");
  await expect(app.getByTestId("lol-today-modes")).toContainText("单双排 1 局 1 胜");
  await expect(app.getByTestId("lol-card-foot").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "client");
  await expect(root).not.toContainText("突破口");
  await expect(root).not.toContainText("风险");
  await screenshot(page, { path: shot(testInfo, "today-card") });

  // 又打了一局（赢了），回到房间：同一个 key 再推一次，原地重画，App 不重新加载
  await app.evaluate(() => { (window as unknown as { __mark: number }).__mark = 7; });
  await page.evaluate(async (key) => {
    const t = window.lolPostTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "today", key, held: false, alert: true, message: t.today(6) });
  }, TODAY_KEY);
  await expect(app.getByTestId("lol-today-head")).toContainText("今天 6 局 · 3 胜 3 负", { timeout: 10_000 });
  await expect(app.getByTestId("lol-today-streak")).toHaveText("上一局赢了");
  await expect(app.getByTestId("lol-today-recent").locator(".rg")).toHaveCount(6);
  expect(await app.evaluate(() => (window as unknown as { __mark?: number }).__mark)).toBe(7);
  await expect(page.getByTestId("push-item")).toHaveCount(1);
});

test("the today card fits a narrow window (the floating window's mini window is about 360 px)", async ({ page }, testInfo) => {
  await open(page, "today");
  const card = pushAt(page, TODAY_KEY);
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "today");
  await expect(app.getByTestId("lol-today-head")).toBeVisible();
  await narrow(app, "lol-app-today");
  for (const id of ["lol-today-head", "lol-today-lost", "lol-today-modes"]) {
    const el = app.getByTestId(id);
    expect(await el.evaluate((e) => e.scrollWidth <= e.clientWidth + 1), id).toBe(true);
  }
  // 再窄一点（小窗最窄的时候）照样不溢出：小头像、模式那一行换行
  await narrow(app, "lol-app-today", 300);
  await screenshot(page, { path: shot(testInfo, "today-card-narrow") });
});

test("the full match details page lists each team's augments under its players (icon + name)", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-lol-open").click();
  const frame = page.getByTestId("plugin-page-lol").getByTestId("mcp-app-frame");
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const lol = page.getByTestId("plugin-page-lol").frameLocator('[data-testid="mcp-app-iframe"]').frameLocator("iframe");
  await lol.getByTestId("lol-app-tab-player").click();
  await lol.getByTestId("lol-player-game").first().click();
  await expect(lol.getByTestId("lol-game-player")).toHaveCount(10);
  // 每队下面一块“海克斯”：一人一行，示例里每人 1–3 个
  const blocks = lol.getByTestId("lol-game-augments");
  await expect(blocks).toHaveCount(2);
  await expect(blocks.first().getByTestId("lol-game-augment-row")).toHaveCount(5);
  await expect(blocks.first()).toContainText("泰坦的坚决");
  await expect(lol.getByTestId("lol-game-team").first()).toContainText("海克斯");
  await lol.getByTestId("lol-game-team").nth(1).scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "game-page-augments") });
});
