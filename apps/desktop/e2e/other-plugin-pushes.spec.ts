import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { screenshot } from "./screenshot.cjs";

// 同传、游戏语音、网易云音乐的推送（ADR 043 D11，B 轮第 4 条）：
// - 同传“这局的同传”：一局一张卡片，每翻好一句原地加一行（译文、原话一行小字），推送的一行是最新的那句；
// - 游戏语音“她对队友说了”：上一句说完后 2 分钟内的几句一张卡片；
// - 网易云音乐“登录过期”：卡片上的“登录网易云音乐”调 music.login_start（不打开右侧栏），登好了原地变成“已登录”；
// - 三个插件的插件页「设置 → 推送」各列出自己的那一条。
// 聊天那一段用 @tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件（照 push.spec.ts），不接任何 AI；插件的命令交给
// 浏览器预览的 pluginsMock，只有 music.login_start 由这里回答并记下来。合并规则、key、事件由各插件的 Rust 测试管。

interface Bridge {
  calls: { tool: string; args: Record<string, unknown> }[];
  push(event: unknown): Promise<void>;
  notices: Record<string, { role: "notice"; atMs: number; text: string; push: Record<string, unknown> }>;
}
declare global {
  interface Window {
    otherPushTest: Bridge;
  }
}

const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round-b-plugins/${name}.png`, import.meta.url));

const T = 1_800_000_000_000;
const LINES_KEY = `lines:${T - 300_000}`;
const SAID_KEY = `said:${T - 200_000}`;
const LOGIN_KEY = `login:${T - 100_000}`;

async function openChat(page: Page) {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.route("**/__other-push-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__other-push-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, LINES_KEY, SAID_KEY, LOGIN_KEY }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    // i18n-ignore-start：示例对话
    const notice = (plugin: string, pluginName: string, trigger: string, key: string, title: string, uri: string, atMs: number,
      data: unknown, line?: string) => ({
      role: "notice" as const, text: `${pluginName} 推送：${title}`, atMs,
      push: {
        plugin, pluginName, trigger, title, key,
        card: { title, ui: { uri }, structuredContent: data },
        ...(line ? { line: { cells: [{ text: line }] } } : {}),
      },
    });
    const lines = notice("interpreter", "同传", "lines", LINES_KEY, "这局的同传", "ui://kotone/interpreter/lines", T - 300_000, {
      target: "中文", startedAtMs: T - 300_000, total: 2,
      lines: [
        { text: "敌人在红房子后面", heard: "Enemy behind the red house", app: "TslGame.exe", atMs: T - 300_000 },
        { text: "我需要医疗包", heard: "I need a medkit", atMs: T - 280_000 },
      ],
    }, "我需要医疗包");
    const said = notice("game-voice", "游戏语音", "said", SAID_KEY, "她对队友说了", "ui://kotone/game-voice/said", T - 200_000, {
      lines: [{ text: "我去拿龙", atMs: T - 200_000 }, { text: "等我一下", atMs: T - 150_000 }],
    });
    const login = notice("music", "网易云音乐", "login", LOGIN_KEY, "登录过期", "ui://kotone/music/login", T - 100_000, { stage: "expired" });
    const conversation = {
      version: 1, id: "c1", title: "这局", createdAtMs: T - 600_000, updatedAtMs: T - 100_000,
      messages: [
        { role: "user", text: "帮我盯着队友说的话", source: "chat", atMs: T - 600_000 },
        { role: "assistant", text: "交给我。", atMs: T - 599_000 },
        lines, said, login,
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c1", running: null, notReadyReason: null,
      conversations: [{ id: "c1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    const bridge = {
      calls: [] as { tool: string; args: Record<string, unknown> }[],
      notices: { lines, said, login },
      async push(event: unknown) {
        await emit("kotone://push", event);
      },
    };
    Object.assign(window, { otherPushTest: bridge });
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
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }]; // i18n-ignore
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
        case "plugins_list": return plugins.listPlugins();
        case "plugin_call": {
          const { plugin, tool, arguments: toolArgs } = a as { plugin: string; tool: string; arguments: Record<string, unknown> };
          if (tool === "music.login_start") {
            bridge.calls.push({ tool, args: toolArgs });
            return { content: { provider: "netease", account: null, login: { stage: "waiting", qrSvg: null, message: null } } };
          }
          return plugins.pluginCall(plugin, tool, toolArgs);
        }
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, LINES_KEY, SAID_KEY, LOGIN_KEY });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);

test("the interpreter push is one card per game with the translation and what was heard, updated in place line by line", async ({ page }) => {
  await openChat(page);
  const item = pushAt(page, LINES_KEY);
  await expect(item.getByTestId("push-from")).toContainText("同传 推送");
  await expect(item.getByTestId("push-title")).toHaveText("这局的同传");
  // 推送的一行就是最新的那句译文（悬浮窗的气泡里显示的也是它）
  await expect(item.getByTestId("push-cell")).toHaveText("我需要医疗包");
  const card = item.getByTestId("interpreter-lines-card");
  await expect(card.getByTestId("interpreter-lines-summary")).toHaveText("2 句 · 译成中文");
  const rows = card.getByTestId("interpreter-line");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByTestId("interpreter-line-text")).toHaveText("敌人在红房子后面");
  await expect(rows.nth(0).getByTestId("interpreter-line-heard")).toHaveText("原话（TslGame.exe）：Enemy behind the red house");
  await expect(rows.nth(1).getByTestId("interpreter-line-heard")).toHaveText("原话：I need a medkit");
  await item.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("interpreter-lines-card") });

  // 又翻好一句：同一个 key 原地更新，还是一张，多了一行，推送的一行换成这一句
  await page.evaluate(async (LINES_KEY) => {
    const t = window.otherPushTest;
    const message = structuredClone(t.notices.lines);
    const push = message.push as { card: { structuredContent: { total: number; lines: unknown[] } }; line: { cells: { text: string }[] } };
    push.card.structuredContent.total = 3;
    push.card.structuredContent.lines.push({ text: "他们从左边绕过来了", heard: "They're flanking left", atMs: message.atMs + 60_000 }); // i18n-ignore
    push.line = { cells: [{ text: "他们从左边绕过来了" }] }; // i18n-ignore
    await t.push({ kind: "updated", conversationId: "c1", plugin: "interpreter", trigger: "lines", key: LINES_KEY, held: false, alert: false, message });
  }, LINES_KEY);
  await expect(pushAt(page, LINES_KEY)).toHaveCount(1);
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(2).getByTestId("interpreter-line-text")).toHaveText("他们从左边绕过来了");
  await expect(card.getByTestId("interpreter-lines-summary")).toHaveText("3 句 · 译成中文");
  await expect(item.getByTestId("push-cell")).toHaveText("他们从左边绕过来了");

  // 卡片本身点了在右侧栏打开（同一个组件）
  await item.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", "ui://kotone/interpreter/lines");
  await expect(sidebar.getByTestId("interpreter-line")).toHaveCount(3);
});

test("the game voice push shows what she said to teammates, one line per sentence", async ({ page }) => {
  await openChat(page);
  const item = pushAt(page, SAID_KEY);
  await expect(item.getByTestId("push-from")).toContainText("游戏语音 推送");
  await expect(item.getByTestId("push-title")).toHaveText("她对队友说了");
  const card = item.getByTestId("game-voice-said-card");
  await expect(card.getByTestId("game-voice-said-to")).toHaveText("她在游戏语音里对队友说了：");
  const rows = card.getByTestId("game-voice-said-line");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("我去拿龙");
  await expect(rows.nth(1)).toContainText("等我一下");
  await item.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("game-voice-said-card") });
});

test("the NetEase login-expired card's button starts the official login without opening the sidebar, then turns into logged in, in place", async ({ page }) => {
  await openChat(page);
  const item = pushAt(page, LOGIN_KEY);
  await expect(item.getByTestId("push-from")).toContainText("网易云音乐 推送");
  await expect(item.getByTestId("push-title")).toHaveText("登录过期");
  const card = item.getByTestId("music-login-push");
  await expect(card).toHaveAttribute("data-state", "expired");
  await expect(card.getByTestId("music-login-push-expired")).toContainText("登录过期了");
  const start = card.getByTestId("music-login-push-start");
  await expect(start).toHaveText("登录网易云音乐");
  await item.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("music-login-expired-card") });

  await start.click();
  await expect.poll(() => page.evaluate(() => window.otherPushTest.calls.map((c) => c.tool))).toEqual(["music.login_start"]);
  await expect(page.getByTestId("right-sidebar").getByTestId("sidebar-plugin-page")).toHaveCount(0);

  // 登好了：后端原地更新这张卡片
  await page.evaluate(async (LOGIN_KEY) => {
    const t = window.otherPushTest;
    const message = structuredClone(t.notices.login);
    (message.push as { card: { structuredContent: unknown } }).card.structuredContent = { stage: "done", nickname: "小琴" }; // i18n-ignore
    await t.push({ kind: "updated", conversationId: "c1", plugin: "music", trigger: "login", key: LOGIN_KEY, held: false, alert: false, message });
  }, LOGIN_KEY);
  await expect(pushAt(page, LOGIN_KEY)).toHaveCount(1);
  await expect(card).toHaveAttribute("data-state", "done");
  await expect(card.getByTestId("music-login-push-done")).toHaveText("已登录网易云音乐：小琴");
  await expect(card.getByTestId("music-login-push-start")).toHaveCount(0);
  await screenshot(page, { path: shot("music-login-done-card") });
});

test("each plugin page lists its push in its settings, on by default", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  for (const [plugin, id, title, forms] of [
    ["interpreter", "lines", "这局的同传", ["卡片", "一行"]],
    ["game-voice", "said", "她对队友说了", ["卡片"]],
    ["music", "login", "登录过期", ["卡片"]],
  ] as [string, string, string, string[]][]) {
    await page.getByTestId("settings-nav-plugins").click();
    await page.getByTestId(`plugins-card-${plugin}-open`).click();
    await page.getByTestId("plugin-settings-button").click();
    const row = page.getByTestId(`plugin-${plugin}-push-${id}-row`);
    await expect(row).toContainText(title);
    await expect(row.getByTestId("plugin-push-form")).toHaveText(forms);
    await expect(page.getByTestId(`plugin-${plugin}-push-${id}`)).toBeChecked();
    if (plugin === "interpreter") await screenshot(page, { path: shot("interpreter-pushes-setting") });
  }
});
