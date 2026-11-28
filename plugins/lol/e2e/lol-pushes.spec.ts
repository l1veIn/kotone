// 英雄联盟原来的两个覆盖层改成了推送（ADR 043 D7）：
// - 选英雄：推一张卡片（插件的 MCP App 的一页 `ui://kotone/lol/champ-select`），聊天里是它；同一次选英雄再推一次时原地更新
//   （App 收到新的数据重画，不重新加载）；选英雄结束时标“结束了”。窄的时候（悬浮窗的小窗约 360 px）也放得下。
// - 海克斯三选一要用本地文字识别：没装时推一张卡片问要不要下载（`ui://kotone/lol/augment-ocr`），下载 / 不用是只给页面的工具
//   `lol.augment_ocr`；一行三格的样子由本体的推送测试（apps/desktop/e2e/push.spec.ts）管。
// 用 @tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件，插件的命令交给浏览器预览的 pluginsMock，卡片里是真的 App
// （vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。由 apps/desktop 的 Playwright 一起跑（plugins 项目）。
import { expect, test, type Frame, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

// 测试页是 Playwright 现给的（route.fulfill），Chrome 按“本地网络访问”的规矩会挡掉它去连回环地址上的 MCP App 代理；
// 这个文件里关掉这项检查（真的 Kotone 窗口里页面本来就在本机）。和 push.spec.ts 一样。
test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const SELECT_KEY = "champ-select:9100001";
const OCR_KEY = "augment-ocr";
const CHAMP_SELECT = "ui://kotone/lol/champ-select";
const AUGMENT_OCR = "ui://kotone/lol/augment-ocr";

/** 截图放在仓库的 target/plugin-shots/round8/（实施记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round8", `${name}.png`);

interface Bridge {
  /** 页面调过的插件工具（mockIPC 的 plugin_call）。 */
  calls: { plugin: string; tool: string }[];
  push(event: unknown): Promise<void>;
  /** 选英雄卡片那条 Notice：`me` 是我现在的英雄（重随以后换了），`ended` 选英雄结束了。 */
  select(me?: string, ended?: boolean): unknown;
  ocr(state: string): unknown;
}
declare global {
  interface Window {
    lolPushTest: Bridge;
  }
}

/** 一段对话：一条“我在吗 / 在的”，后面是推来的选英雄卡片（和要不要下载识别模型的卡片）。 */
async function open(page: Page, options: { ocr?: boolean; width?: number } = {}) {
  await page.setViewportSize({ width: options.width ?? 1280, height: 900 });
  await page.route("**/__lol-push-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__lol-push-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, SELECT_KEY, OCR_KEY, CHAMP_SELECT, AUGMENT_OCR, options }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    const { sampleChampSelect } = await import("/src/lib/ipc/lolStatsMock.ts");
    // i18n-ignore-start：示例对话
    const select = (me = "艾瑞莉娅", ended = false) => ({
      role: "notice", text: "英雄联盟 推送：选英雄", atMs: T - 30_000,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "champ-select", title: "选英雄", key: SELECT_KEY,
        card: { title: "选英雄", ui: { uri: CHAMP_SELECT }, structuredContent: sampleChampSelect("mayhem", me, ended) },
        ...(me === "艾瑞莉娅" && !ended ? {} : { updatedAtMs: T }),
      },
    });
    const ocr = (state: string) => ({
      role: "notice", text: "英雄联盟 推送：文字识别模型", atMs: T - 20_000,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "augment-pick", title: "文字识别模型", key: OCR_KEY,
        card: { title: "文字识别模型", ui: { uri: AUGMENT_OCR }, structuredContent: { state, model: "PP-OCRv5 mobile", sizeMb: 21 } },
      },
    });
    const conversation = {
      version: 1, id: "c-1", title: "这局", createdAtMs: T - 86_400_000, updatedAtMs: T - 20_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        select(),
        ...(options.ocr ? [ocr("ask")] : []),
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c-1", running: null, notReadyReason: null,
      conversations: [{ id: "c-1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    const bridge = {
      calls: [] as { plugin: string; tool: string }[],
      async push(event: unknown) {
        await emit("kotone://push", event);
      },
      select, ocr,
    };
    Object.assign(window, { lolPushTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      if (command === "plugin_call") bridge.calls.push({ plugin: (a as { plugin: string }).plugin, tool: (a as { tool: string }).tool });
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
  }, { T, SELECT_KEY, OCR_KEY, CHAMP_SELECT, AUGMENT_OCR, options });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);

/** 卡片里那一层 App（沙盒代理页里面那一层，地址是 /<密钥>/app/lol/<页面>）。 */
async function appFrame(page: Page, which: string): Promise<Frame> {
  const pattern = new RegExp(`/app/lol/${which}$`);
  await expect.poll(() => page.frames().filter((f) => pattern.test(f.url())).length, { timeout: 15_000 }).toBe(1);
  return page.frames().find((f) => pattern.test(f.url()))!;
}

test("the champ-select push is an App card in the chat: team grades, the bench above you, the best swap ringed", async ({ page }, testInfo) => {
  await open(page);
  const card = pushAt(page, SELECT_KEY);
  await expect(card.getByTestId("push-from")).toContainText("英雄联盟 推送");
  await expect(card.getByTestId("push-title")).toHaveText("选英雄");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(`[data-testid="mcp-app-frame"][data-uri="${CHAMP_SELECT}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "champ-select");
  const root = app.getByTestId("lol-app-champ-select");
  await expect(root).toHaveAttribute("data-mode", "mayhem");
  await expect(root).toHaveAttribute("data-ended", "false");
  // 我方 5 个：评级小牌 + 胜率 + 第几 / 共几；第一行是我
  const rows = app.getByTestId("lol-champ-select-row");
  await expect(rows).toHaveCount(5);
  await expect(rows.first()).toHaveAttribute("data-me", "true");
  await expect(rows.first()).toContainText("艾瑞莉娅");
  await expect(rows.first()).toContainText("你");
  await expect(rows.first()).toContainText("52.1%");
  await expect(rows.first()).toContainText("第 31/172");
  expect(await rows.evaluateAll((els) => els.map((e) => e.getAttribute("data-grade")))).toEqual(["A", "S", "A", "C", "B"]);
  // 备选：按胜率排，比我高的两个标 ↑，最高的那个用墨线圈出来
  await expect(root).toContainText("胜率比你现在的英雄高");
  const seats = app.getByTestId("lol-champ-select-seat");
  await expect(seats).toHaveCount(4);
  await expect(seats.first()).toContainText("斯莫德");
  await expect(seats.first()).toHaveAttribute("data-best", "true");
  await expect(seats.first()).toContainText("↑");
  await expect(seats.nth(1)).toHaveAttribute("data-better", "true");
  await expect(seats.nth(2)).toHaveAttribute("data-better", "false");
  await expect(seats.nth(2)).not.toContainText("↑");
  expect(await seats.first().evaluate((e) => getComputedStyle(e).boxShadow)).not.toBe("none");
  expect(await seats.nth(1).evaluate((e) => getComputedStyle(e).boxShadow)).toBe("none");
  await expect(root).toContainText("腾讯 101");
  await screenshot(page, { path: shot(testInfo, "chat-champ-select-card") });
});

test("the same champ select pushed again redraws the card in place (a reroll), without reloading the App; the last push marks it over", async ({ page }) => {
  await open(page);
  const card = pushAt(page, SELECT_KEY);
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "champ-select");
  await expect(app.getByTestId("lol-champ-select-row").first()).toContainText("艾瑞莉娅");
  await app.evaluate(() => { (window as unknown as { __mark: number }).__mark = 7; });
  // 我重随成了斯莫德：同一个 key 再推一次（宿主说“更新了”）
  await page.evaluate(async (key) => {
    const t = window.lolPushTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "champ-select", key, held: false, alert: false,
      message: t.select("斯莫德") }); // i18n-ignore
  }, SELECT_KEY);
  await expect(app.getByTestId("lol-champ-select-row").first()).toContainText("斯莫德", { timeout: 10_000 });
  await expect(app.getByTestId("lol-champ-select-row").first()).toHaveAttribute("data-grade", "S");
  // 备选里没有比我高的了
  await expect(app.locator('[data-testid="lol-champ-select-seat"][data-better="true"]')).toHaveCount(0);
  expect(await app.evaluate(() => (window as unknown as { __mark?: number }).__mark)).toBe(7);
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  // 选英雄结束了：最后一次，卡片标上“结束了”
  await page.evaluate(async (key) => {
    const t = window.lolPushTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "champ-select", key, held: false, alert: false,
      message: t.select("斯莫德", true) }); // i18n-ignore
  }, SELECT_KEY);
  await expect(app.getByTestId("lol-app-champ-select")).toHaveAttribute("data-ended", "true", { timeout: 10_000 });
  await expect(app.getByTestId("lol-app-champ-select")).toContainText("选英雄结束了");
  await expect(page.getByTestId("push-item")).toHaveCount(1);
});

test("the champ-select card fits a narrow window (the floating window's mini window is about 360 px)", async ({ page }, testInfo) => {
  await open(page);
  const card = pushAt(page, SELECT_KEY);
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "champ-select");
  await expect(app.getByTestId("lol-champ-select-row")).toHaveCount(5);
  // App 那一层窄到 360 px：不横着溢出，名字和胜率都在框里
  await app.evaluate(() => { document.body.style.width = "360px"; });
  const root = app.getByTestId("lol-app-champ-select");
  expect(await root.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  const seats = app.getByTestId("lol-champ-select-seat");
  // 换行由页面里量宽度的那一下决定，机器忙时晚一帧：等它排好再比
  await expect
    .poll(async () => {
      const [a, b] = [await seats.nth(0).boundingBox(), await seats.nth(1).boundingBox()];
      return b!.y - a!.y;
    })
    .toBeGreaterThan(4);
  const rate = app.getByTestId("lol-champ-select-row").first().locator(".rate");
  expect(await rate.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await screenshot(page, { path: shot(testInfo, "chat-champ-select-narrow") });
});

test("no local text recognition model: a card asks to download it (about 21 MB); 下载 calls lol.augment_ocr through the lol plugin", async ({ page }, testInfo) => {
  await open(page, { ocr: true });
  const card = pushAt(page, OCR_KEY);
  await expect(card.getByTestId("push-title")).toHaveText("文字识别模型");
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "augment-ocr");
  const root = app.getByTestId("lol-app-augment-ocr");
  await expect(root).toHaveAttribute("data-state", "ask");
  await expect(root).toContainText("海克斯评分要用本地的文字识别模型（约 21 MB），要下载吗？");
  await expect(root).toContainText("不会改你在「设置 → 模型 → 文字识别」里的选择");
  await screenshot(page, { path: shot(testInfo, "chat-augment-ocr-ask") });
  await app.getByTestId("lol-augment-ocr-download").click();
  await expect.poll(async () => (await page.evaluate(() => window.lolPushTest.calls)).filter((c) => c.tool === "lol.augment_ocr"))
    .toEqual([{ plugin: "lol", tool: "lol.augment_ocr" }]);
  // 预览里马上算下好了（真的是在后台下，插件下完再原地更新这张卡片）
  await expect(root).toHaveAttribute("data-state", "ready");
  await expect(root).toContainText("下好了");
  // 插件后来推来的更新（不用了）：卡片跟着变
  await page.evaluate(async (key) => {
    const t = window.lolPushTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "augment-pick", key, held: false, alert: false, message: t.ocr("declined") });
  }, OCR_KEY);
  await expect(root).toHaveAttribute("data-state", "declined", { timeout: 10_000 });
  await expect(root).toContainText("以后想用");
});
