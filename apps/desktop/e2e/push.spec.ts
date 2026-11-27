import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openPlugin, openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 推送（ADR 043 D3–D5；以前叫自动卡片，ADR 039 D3）：用 @tauri-apps/api/mocks 模拟 agent_* 命令和
// kotone://agent-event、kotone://push 事件，不接任何 AI。插件的命令（plugins_list、plugin_app……）交给浏览器预览的 pluginsMock，
// 英雄联盟的卡片是真的 MCP App（vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。
// 后端怎么存、什么时候追加给她的那句、原地更新怎么等正在跑的那一轮，由 Rust 单元测试（kotone-agent push、桌面端 when_free / turn）管。
// 悬浮窗（推送的一行、小窗；原来的消息提示）在 floating.spec.ts。

interface Bridge {
  calls: { command: string; args?: Record<string, unknown> }[];
  event(conversationId: string, event: unknown): Promise<void>;
  push(event: unknown): Promise<void>;
  focus(conversationId: string, key: string): Promise<void>;
  /** 一条新的推送（这局总结），给“她正在回答时来了推送”用。 */
  fresh: unknown;
  /** 对话里那张开局战绩（新的推送格式），`update(name)` 是同一个 key、我那一行改了名字的版本。 */
  opening: { role: "notice"; atMs: number; push: Record<string, unknown> };
  update(name: string): unknown;
  /** 带提醒的更新挪到最后的那条（ADR 043 D5 修订）：和 `update` 一样，时间是挪的那一刻。 */
  moved(name: string, atMs: number): unknown;
}
declare global {
  interface Window {
    pushTest: Bridge;
  }
}

// 测试页是 Playwright 现给的（route.fulfill），Chrome 不知道它来自本机，按“本地网络访问”的规矩挡掉它去连回环地址上的
// MCP App 代理（卡片里的 App 一直在加载）；这个文件里关掉这项检查（真的 Kotone 窗口里页面本来就在本机）。
test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const KEY = "opening:9100000";
const LEGACY_KEY = "opening:9000001";
/** 截图放在仓库的 target/plugin-shots/round8/（实施记录用）。 */
const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round8/${name}.png`, import.meta.url));

/** `held`：agent_held_cards 回什么（排着的推送：新的、排着的原地更新、排着的挪动）。 */
async function open(page: Page, options: { legacy?: boolean; held?: "update" | "move" } = {}) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__push-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__push-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, KEY, LEGACY_KEY, options }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    const { sampleMatch } = await import("/src/lib/ipc/lolMock.ts");
    // i18n-ignore-start：示例对话
    const record = (key: string, title: string, data: unknown) => ({
      plugin: "lol", pluginName: "英雄联盟", trigger: key.startsWith("opening") ? "opening" : "summary",
      title, key, card: { title, ui: { uri: "ui://kotone/lol/match" }, structuredContent: data },
    });
    const opening = { role: "notice" as const, text: "英雄联盟 推送：开局战绩", atMs: T - 60_000, push: record(KEY, "开局战绩", sampleMatch()) };
    const note = {
      role: "user", source: "auto-card", atMs: T - 60_000,
      text: "（系统消息，不是玩家说的话：英雄联盟插件发来一条推送「开局战绩」，玩家在聊天里看得到；要点见附件。）",
      attachments: [{ kind: "data", label: "英雄联盟 · 开局战绩", text: "开局战绩（海克斯大乱斗）。" }],
    };
    // rc.9 写的自动卡片（Notice 的 card）：照样画成卡片
    const legacy = {
      role: "notice", text: "英雄联盟 发来一张卡片：开局战绩", atMs: T - 90_000,
      card: { plugin: "lol", pluginName: "英雄联盟", key: LEGACY_KEY, title: "开局战绩", popup: "对面的战绩到了",
        ui: { uri: "ui://kotone/lol/match" }, structuredContent: sampleMatch() },
    };
    const old = {
      version: 1, id: "c-old", title: "上次的对话", createdAtMs: T - 86_400_000, updatedAtMs: T - 60_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        ...(options.legacy ? [legacy] : []),
        opening, note,
        { role: "user", text: "对面谁最菜", source: "chat", atMs: T - 50_000 },
        { role: "assistant", text: "我看看。", atMs: T - 49_000 },
      ],
    };
    const update = (name: string) => {
      const data = sampleMatch();
      data.allies[0] = { ...data.allies[0], name, riotId: `${name}#1001` };
      return { ...opening, push: { ...record(KEY, "开局战绩", data), updatedAtMs: T } };
    };
    const moved = (name: string, atMs: number) => {
      const m = update(name);
      return { ...m, atMs, push: { ...m.push, updatedAtMs: atMs } };
    };
    // i18n-ignore-end
    const conversations: Record<string, typeof old> = { "c-old": old };
    const state = {
      current: "c-old", running: null as string | null, notReadyReason: null,
      conversations: [{ id: "c-old", title: old.title, createdAtMs: old.createdAtMs, updatedAtMs: old.updatedAtMs, messageCount: old.messages.length }],
    };
    const fresh = {
      role: "notice", text: "英雄联盟 推送：这局总结", atMs: T + 3_000, // i18n-ignore
      push: record("summary:9100000", "这局总结", sampleMatch()), // i18n-ignore
    };
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[], fresh, opening, update, moved,
      async event(conversationId: string, event: unknown) {
        const e = event as { kind: string; message?: unknown };
        if (e.kind === "appended") conversations[conversationId]?.messages.push(e.message as never);
        if (e.kind === "step") state.running = conversationId;
        if (e.kind === "idle") state.running = null;
        await emit("kotone://agent-event", { conversationId, event });
      },
      async push(event: unknown) {
        await emit("kotone://push", event);
      },
      async focus(conversationId: string, key: string) {
        await emit("kotone://auto-card-focus", { conversationId, key });
      },
    };
    Object.assign(window, { pushTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      if (command.startsWith("agent_") || command === "update_settings") bridge.calls.push({ command, args: a });
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversations[(a as { id: string }).id] ?? null);
        case "agent_held_cards": // i18n-ignore-start
          return options.held === "update" ? [update("排着的新名字")] : options.held === "move" ? [moved("排着挪的名字", T + 5_000)] : [];
          // i18n-ignore-end
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
  }, { T, KEY, LEGACY_KEY, options });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);
/** 每一轮里推送的 key（没有推送的轮是空串）：看位置。 */
const turnKeys = (page: Page) => page.getByTestId("agent-turn").evaluateAll((turns) =>
  turns.map((t) => t.querySelector<HTMLElement>('[data-testid="push-item"]')?.dataset.key ?? ""));

test("a pushed card shows in the chat as 英雄联盟 推送, opens in the right sidebar, the note for her stays hidden, and an old auto card still renders", async ({ page }) => {
  await open(page, { legacy: true });
  const card = pushAt(page, KEY);
  await expect(page.getByTestId("push-item")).toHaveCount(2);
  await expect(card.getByTestId("push-from")).toContainText("英雄联盟 推送");
  await expect(card.getByTestId("push-title")).toHaveText("开局战绩");
  await expect(card.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]')).toBeVisible();
  // rc.9 写的自动卡片（Notice 的 card）：照样是一张卡片
  const legacy = pushAt(page, LEGACY_KEY);
  await expect(legacy.getByTestId("push-from")).toContainText("英雄联盟 推送");
  await expect(legacy.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]')).toBeVisible();
  // 给她的那句（来源 auto-card）不显示成玩家的气泡；推送不挂在她的回答下面
  await expect(page.getByTestId("agent-user-message").filter({ hasText: "系统消息" })).toHaveCount(0);
  await expect(card.locator("xpath=ancestor::*[@data-testid='agent-turn']").getByTestId("agent-answer")).toHaveCount(0);
  await card.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("chat-push-card") });

  await card.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-tab-page")).toHaveAttribute("aria-selected", "true");
  await expect(sidebar.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", "ui://kotone/lol/match");
});

test("the same key again updates the card in place: same position, no duplicate, and the App inside gets the new data without reloading", async ({ page }) => {
  await open(page);
  const card = pushAt(page, KEY);
  const frame = card.locator('[data-testid="mcp-app-frame"]');
  await card.scrollIntoViewIfNeeded();
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect.poll(() => page.frames().filter((f) => /\/app\/lol\/match$/.test(f.url())).length, { timeout: 15_000 }).toBe(1);
  const app = page.frames().find((f) => /\/app\/lol\/match$/.test(f.url()))!;
  await expect(app.locator("body")).not.toContainText("改过的名字");
  // App 那一层记一个标记：之后还在 = 没有重新加载
  await app.evaluate(() => { (window as unknown as { __mark: number }).__mark = 42; });
  const before = await turnKeys(page);

  await page.evaluate(async (KEY) => {
    const t = window.pushTest;
    await t.push({ kind: "updated", conversationId: "c-old", plugin: "lol", trigger: "opening", key: KEY, held: false, alert: false,
      message: t.update("改过的名字") }); // i18n-ignore
  }, KEY);
  await expect(app.locator("body")).toContainText("改过的名字", { timeout: 10_000 });
  expect(await app.evaluate(() => (window as unknown as { __mark?: number }).__mark)).toBe(42);
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  expect(await turnKeys(page)).toEqual(before);
  await card.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("chat-push-updated") });

  // 后端把它记进对话以后又来一条 appended（同一个 key）：原地换成存盘的这条，不多一张
  await page.evaluate(async () => {
    const t = window.pushTest;
    await t.event("c-old", { kind: "appended", message: t.update("存盘的名字") }); // i18n-ignore
  });
  await expect(app.locator("body")).toContainText("存盘的名字", { timeout: 10_000 });
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  expect(await turnKeys(page)).toEqual(before);
});

test("an update still waiting in the queue replaces the saved card when the chat loads", async ({ page }) => {
  await open(page, { held: "update" });
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  const keys = await turnKeys(page);
  expect(keys.filter(Boolean)).toEqual([KEY]);
  expect(keys.indexOf(KEY)).toBe(1);
  await pushAt(page, KEY).scrollIntoViewIfNeeded();
  const frame = pushAt(page, KEY).locator('[data-testid="mcp-app-frame"]');
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect.poll(() => page.frames().find((f) => /\/app\/lol\/match$/.test(f.url()))?.locator("body").textContent(), { timeout: 15_000 })
    .toContain("排着的新名字");
});

/** 聊天里某一条在可视区里的上沿（px，相对聊天那一块）：看页面有没有跳。 */
const topIn = (page: Page, text: string) => page.getByTestId("agent-transcript").evaluate((viewport, text) => {
  const el = [...viewport.querySelectorAll<HTMLElement>('[data-testid="agent-turn"]')].find((t) => t.textContent?.includes(text));
  return el ? Math.round(el.getBoundingClientRect().top - viewport.getBoundingClientRect().top) : null;
}, text);
const atBottom = (page: Page) => page.getByTestId("agent-transcript").evaluate((v) => v.scrollHeight - v.scrollTop - v.clientHeight < 2);
/** 等两帧（事件改完对话、页面画完、滚动事件到了）。 */
const settle = (page: Page) => page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

// ADR 043 D5 修订（rc.14 实测：海斗里“你阵亡时对面的装备”一局更新十来次，每次都跳回开局附近）
test("an update with alert moves the card to the bottom (still one card); one without alert stays in place and the chat does not scroll to it", async ({ page }) => {
  await open(page);
  // 开局战绩之后又来了几轮：卡片在上面，页面要往下翻
  await page.evaluate(async (T) => {
    const t = window.pushTest;
    for (let i = 1; i <= 6; i += 1) {
      // i18n-ignore-start：示例对话
      await t.event("c-old", { kind: "appended", message: { role: "user", text: `第 ${i} 个问题`, source: "chat", atMs: T + i * 2_000 } });
      await t.event("c-old", { kind: "appended", message: { role: "assistant", text: `${"这是一段比较长的回答。\n\n".repeat(6)}第 ${i} 个回答`, atMs: T + i * 2_000 + 1_000 } });
      // i18n-ignore-end
    }
  }, T);
  const card = pushAt(page, KEY);
  await expect(page.getByTestId("agent-turn").filter({ hasText: "第 6 个回答" })).toBeVisible();
  const keys = await turnKeys(page);
  expect(keys.indexOf(KEY)).toBe(1);
  expect(keys.length).toBe(9);

  // 玩家往上翻着看（第 3 个问题在最上面，卡片在可视区上面）
  await page.getByTestId("agent-turn").filter({ hasText: "第 3 个问题" }).evaluate((el) => el.scrollIntoView({ block: "start" }));
  await settle(page);
  await expect(card).not.toBeInViewport();
  expect(await atBottom(page)).toBe(false);
  const before = await topIn(page, "第 3 个问题");

  // 不带提醒的更新：原地换内容，位置不动，页面不滚
  await page.evaluate(async (KEY) => {
    const t = window.pushTest;
    await t.push({ kind: "updated", conversationId: "c-old", plugin: "lol", trigger: "opening", key: KEY, held: false, alert: false,
      message: t.update("改过的名字") }); // i18n-ignore
  }, KEY);
  await settle(page);
  expect(await turnKeys(page)).toEqual(keys);
  expect(await topIn(page, "第 3 个问题")).toBe(before);
  await expect(card).not.toBeInViewport();

  // 带提醒的更新：从原处拿掉、放到最后，只有一张；玩家往上翻着看：看着的那一段不动
  await page.evaluate(async ({ KEY, T }) => {
    const t = window.pushTest;
    await t.push({ kind: "moved", conversationId: "c-old", plugin: "lol", trigger: "opening", key: KEY, held: false, alert: true,
      message: t.moved("挪过来的名字", T + 20_000) }); // i18n-ignore
  }, { KEY, T });
  await expect.poll(async () => (await turnKeys(page)).indexOf(KEY)).toBe(8);
  await settle(page);
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  expect((await turnKeys(page)).length).toBe(9);
  expect(await topIn(page, "第 3 个问题")).toBe(before);
  await expect(card).not.toBeInViewport();

  // 玩家翻回最下面，又来了一轮：跟着到最下面；再来一次带提醒的更新：卡片又在最后，页面跟到最下面、看得到它
  await page.getByTestId("agent-transcript").evaluate((v) => { v.scrollTop = v.scrollHeight; v.dispatchEvent(new Event("scroll")); });
  await page.evaluate(async (T) => {
    const t = window.pushTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "第 7 个问题", source: "chat", atMs: T + 30_000 } }); // i18n-ignore
  }, T);
  await expect.poll(async () => (await turnKeys(page)).indexOf(KEY)).toBe(8);
  await expect.poll(() => atBottom(page)).toBe(true);
  await page.evaluate(async ({ KEY, T }) => {
    const t = window.pushTest;
    await t.push({ kind: "moved", conversationId: "c-old", plugin: "lol", trigger: "opening", key: KEY, held: false, alert: true,
      message: t.moved("又挪过来的名字", T + 40_000) }); // i18n-ignore
  }, { KEY, T });
  await expect.poll(async () => (await turnKeys(page)).at(-1)).toBe(KEY);
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  await expect.poll(() => atBottom(page)).toBe(true);
  await expect(card).toBeInViewport();
  // 卡片里的 App 拿到的是最新的数据
  await expect.poll(() => page.frames().find((f) => /\/app\/lol\/match$/.test(f.url()))?.locator("body").textContent(), { timeout: 15_000 })
    .toContain("又挪过来的名字");
  await screenshot(page, { path: shot("chat-push-moved") });
});

test("a move still waiting in the queue puts the card at the bottom when the chat loads", async ({ page }) => {
  await open(page, { held: "move" });
  await expect(page.getByTestId("push-item")).toHaveCount(1);
  await expect.poll(async () => (await turnKeys(page)).at(-1)).toBe(KEY);
  expect((await turnKeys(page)).filter(Boolean)).toEqual([KEY]);
});

test("a push that comes while she is answering shows at the bottom at once and stays after her answer", async ({ page }) => {
  await open(page);
  await page.evaluate(async (T) => {
    const t = window.pushTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "这局怎么样", source: "voice", atMs: T } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "chunk", text: "我看看" });
    // 她正在回答：推送先显示（还没记进对话）
    await t.push({ kind: "posted", conversationId: "c-old", plugin: "lol", trigger: "summary", key: "summary:9100000", held: true, alert: false, message: t.fresh });
  }, T);
  const items = page.getByTestId("push-item");
  await expect(items).toHaveCount(2);
  await expect(page.getByTestId("agent-turn").last().getByTestId("push-item")).toHaveAttribute("data-key", "summary:9100000");
  // 她这一轮还在往下写：流式的字留在她那一轮里，不挂到推送下面
  await expect(page.locator('[data-turn-active="true"]').getByTestId("agent-answer")).toContainText("我看看");
  await page.evaluate(async (T) => {
    const t = window.pushTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "亚索最近 6 连败。", atMs: T + 4_000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
    // 这一轮结束：后端把推送和给她的那句记进对话（同一条，不重复显示）
    await t.event("c-old", { kind: "appended", message: t.fresh });
    await t.event("c-old", { kind: "appended", message: { role: "user", source: "auto-card", text: "（系统消息……）", atMs: T + 3_000 } });
  }, T);
  await expect(items).toHaveCount(2);
  const turns = page.getByTestId("agent-turn");
  await expect(turns.last().getByTestId("push-item")).toHaveAttribute("data-key", "summary:9100000");
  await expect(turns.nth(-2).getByTestId("agent-answer")).toContainText("亚索最近 6 连败。");
  await expect(page.getByTestId("agent-user-message").filter({ hasText: "系统消息" })).toHaveCount(0);
});

test("a line push and a voice push are a small text row in the chat, updated in place too", async ({ page }) => {
  await open(page);
  // i18n-ignore-start：示例推送
  const line = (cells: { text: string; tone?: string }[], said: string) => ({
    role: "notice", text: "英雄联盟 推送：海克斯三选一", atMs: T + 10_000,
    push: { plugin: "lol", pluginName: "英雄联盟", trigger: "augment-pick", title: "海克斯三选一", key: "pick:1",
      line: { cells }, voice: ["中间那个", "选中间"], said },
  });
  await page.evaluate(async (message) => {
    await window.pushTest.push({ kind: "posted", conversationId: "c-old", plugin: "lol", trigger: "augment-pick", key: "pick:1", held: false, alert: false, message });
  }, line([{ text: "可选", tone: "ok" }, { text: "首选", tone: "good" }, { text: "可刷新", tone: "bad" }], "中间那个"));
  const row = pushAt(page, "pick:1");
  await expect(row.getByTestId("push-title")).toHaveText("海克斯三选一");
  await expect(row.getByTestId("push-cell")).toHaveText(["可选", "首选", "可刷新"]);
  await expect(row.getByTestId("push-cell").nth(1)).toHaveAttribute("data-tone", "good");
  await expect(row.getByTestId("push-voice")).toHaveText("中间那个");
  await expect(row.getByTestId("plugin-card")).toHaveCount(0);
  await row.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("chat-push-line") });
  // 刷新了一张：原地换那一格
  await page.evaluate(async (message) => {
    await window.pushTest.push({ kind: "updated", conversationId: "c-old", plugin: "lol", trigger: "augment-pick", key: "pick:1", held: false, alert: false, message });
  }, line([{ text: "可选", tone: "ok" }, { text: "首选", tone: "good" }, { text: "首选", tone: "good" }], "中间那个"));
  // i18n-ignore-end
  await expect(row.getByTestId("push-cell")).toHaveText(["可选", "首选", "首选"]);
  await expect(page.getByTestId("push-item")).toHaveCount(2);
});

test("an automation's push() is a line from 自动化, not from a plugin (ADR 045 D2)", async ({ page }) => {
  await open(page);
  // i18n-ignore-start：示例推送（后端合成的自动化推送：plugin = automation，trigger = 自动化 id，key = 执行记录 id + 第几条）
  const message = {
    role: "notice", text: "自动化 推送：买眼提醒", atMs: T + 10_000,
    push: { plugin: "automation", pluginName: "自动化", trigger: "auto-3", title: "买眼提醒", key: "run-7-1", line: { cells: [{ text: "记得买真眼" }] } },
  };
  // i18n-ignore-end
  await page.evaluate(async (message) => {
    await window.pushTest.push({ kind: "posted", conversationId: "c-old", plugin: "automation", trigger: "auto-3", key: "run-7-1", held: false, alert: false, message });
  }, message);
  const row = pushAt(page, "run-7-1");
  await expect(row.getByTestId("push-from")).toContainText("自动化 推送");
  await expect(row.getByTestId("push-title")).toHaveText("买眼提醒");
  await expect(row.getByTestId("push-cell")).toHaveText(["记得买真眼"]);
});

test("clicking a card in the floating window opens the chat at the pushed card", async ({ page }) => {
  await open(page);
  // 玩家在别的页面（交互设置）
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("assistant-chat")).not.toBeVisible();
  await page.evaluate(async (KEY) => { await window.pushTest.focus("c-old", KEY); }, KEY);
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const card = pushAt(page, KEY);
  await expect(card).toBeInViewport();
  await expect(card).toHaveClass(/flashing/);
});

test("交互 has no master switch any more: only 推送时弹出小窗 (in 悬浮窗), which still writes agent.autoCards.popup", async ({ page }) => {
  await open(page);
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("bubble-settings")).toBeVisible();
  await expect(page.getByTestId("auto-cards-enabled")).toHaveCount(0);
  await expect(page.getByTestId("auto-card-settings")).toHaveCount(0);
  await expect(page.getByTestId("push-popup")).toBeChecked();
  await page.locator("#settings-reply-bubble").scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("settings-push-popup") });
  await page.getByTestId("push-popup").click();
  await expect(page.getByTestId("push-popup")).not.toBeChecked();
  const patches = await page.evaluate(() => window.pushTest.calls.filter((c) => c.command === "update_settings").map((c) => c.args?.patch));
  expect(patches).toContainEqual({ agent: { autoCards: { enabled: true, popup: false } } });
});

test("the plugin page lists its pushes under 设置 → 推送, one switch each, and the switch writes plugins.lol.pushes", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__push-plugin-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__push-plugin-test#/settings?onboarding=never");
  // 旧配置：“自动卡片”总开关关着 → 带卡片的推送算关，直到玩家动了那一条（浏览器预览，不模拟 IPC）
  await page.evaluate(async () => {
    const browser = await import("/src/lib/ipc/browserMock.ts");
    await browser.updateSettings({ agent: { autoCards: { enabled: false, popup: true } } });
    await import("/src/main.ts");
  });
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  const pluginPage = await openPlugin(page, "lol");
  await pluginPage.getByTestId("plugin-settings-button").click();
  const section = page.getByTestId("plugin-lol-pushes");
  await expect(section).toBeVisible();
  const opening = section.getByTestId("plugin-lol-push-opening-row");
  await expect(opening).toContainText("开局战绩");
  await expect(opening.getByTestId("plugin-push-form")).toHaveText(["卡片"]);
  await expect(opening.getByTestId("plugin-push-when")).toContainText("进加载");
  await expect(opening.getByTestId("plugin-push-what")).toContainText("双方战绩");
  await expect(section.getByTestId("plugin-lol-push-summary-row")).toContainText("这局总结");
  await expect(section).toContainText("自动化");
  await expect(page.getByTestId("plugin-lol-push-opening")).not.toBeChecked();
  await expect(page.getByTestId("plugin-lol-push-summary")).not.toBeChecked();
  await section.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("plugin-pushes") });

  const stored = () => page.evaluate(async () => {
    const browser = await import("/src/lib/ipc/browserMock.ts");
    return (await browser.getSettings()).plugins?.lol?.pushes ?? null;
  });
  await page.getByTestId("plugin-lol-push-opening").click();
  await expect(page.getByTestId("plugin-lol-push-opening")).toBeChecked();
  await expect(page.getByTestId("plugin-lol-push-summary")).not.toBeChecked();
  expect(await stored()).toEqual({ opening: true });
  await page.getByTestId("plugin-lol-push-opening").click();
  await expect(page.getByTestId("plugin-lol-push-opening")).not.toBeChecked();
  expect(await stored()).toEqual({ opening: false });

  // 插件关着：开关灰着、位置不变
  await page.getByTestId("plugin-lol-enabled").click();
  await expect(page.getByTestId("plugin-lol-push-summary")).toBeDisabled();
  await expect(page.getByTestId("plugin-lol-push-opening")).not.toBeChecked();
});
