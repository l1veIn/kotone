import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { screenshot } from "./screenshot.cjs";

// 聊天里的卡片不在里面滚（2026-11-27 rc.14 试玩：“卡片有最大高度，然后就出现了卡片内部一个滚动……这种滚动嵌套非常不好，
// 很卡而且吃性能”）：卡片里的 MCP App（McpAppFrame，inline）多高显示多高，iframe 是 App 的整个高度、不出滚动条，滚轮落在
// 卡片上时滚的是聊天页；只有超过防呆上限（2000）才截，底部渐隐 +“展开”。悬浮窗的小窗里（截到列表能放下的、“在 Kotone 里
// 看完整的”）在 floating.spec.ts。用 @tauri-apps/api/mocks 模拟 agent_* 命令；英雄联盟的卡片是真的 MCP App
// （vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。

test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const KEY = "opening:9100000";
const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/roundB/${name}.png`, import.meta.url));

async function open(page: Page) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__card-height-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__card-height-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, KEY }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    const { sampleMatch } = await import("/src/lib/ipc/lolMock.ts");
    // i18n-ignore-start：示例对话
    const opening = { role: "notice", text: "英雄联盟 推送：开局战绩", atMs: T - 60_000,
      push: { plugin: "lol", pluginName: "英雄联盟", trigger: "opening", title: "开局战绩", key: KEY,
        card: { title: "开局战绩", ui: { uri: "ui://kotone/lol/match" }, structuredContent: sampleMatch() } } };
    const filler = Array.from({ length: 6 }, (_, i) => [
      { role: "user", text: `第 ${i} 句`, source: "chat", atMs: T - 50_000 + i * 2_000 },
      { role: "assistant", text: `好的，第 ${i} 句。`, atMs: T - 49_000 + i * 2_000 },
    ]).flat();
    const conversation = {
      version: 1, id: "c1", title: "这局", createdAtMs: T - 600_000, updatedAtMs: T,
      messages: [{ role: "user", text: "在吗", source: "chat", atMs: T - 600_000 }, { role: "assistant", text: "在的。", atMs: T - 599_000 },
        opening, ...filler],
    };
    // i18n-ignore-end
    const state = { current: "c1", running: null, notReadyReason: null,
      conversations: [{ id: "c1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: T, messageCount: conversation.messages.length }] };
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return (a as { id: string }).id === "c1" ? structuredClone(conversation) : null;
        case "agent_held_cards": return [];
        case "automation_runs": return { runs: [] };
        case "usage_conversation": return null;
        case "list_connections": return [{ id: "test-ai", displayName: "test", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
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
  }, { T, KEY });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

/** 卡片外面那一块、iframe、App 自己的高度，App 里能不能滚。 */
const sizes = (page: Page) => page.locator(`[data-testid="push-item"][data-key="${KEY}"] [data-testid="mcp-app-frame"]`).evaluate((el) => {
  const box = el.firstElementChild as HTMLElement;
  const iframe = el.querySelector("iframe")!;
  return { box: box.getBoundingClientRect().height, iframe: iframe.getBoundingClientRect().height, clipped: el.dataset.clipped === "true",
    scrolling: iframe.getAttribute("scrolling") };
});

async function readyApp(page: Page) {
  const card = page.locator(`[data-testid="push-item"][data-key="${KEY}"]`);
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator('[data-testid="mcp-app-frame"]')).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect.poll(() => page.frames().some((f) => /\/app\/lol\/match$/.test(f.url())), { timeout: 15_000 }).toBe(true);
  return { card, app: page.frames().find((f) => /\/app\/lol\/match$/.test(f.url()))! };
}

test("a card in the chat is as tall as its App: no inner scroll bar, and the wheel over it scrolls the chat", async ({ page }) => {
  await open(page);
  const { card, app } = await readyApp(page);
  // App 报了高度：外面那一块、iframe 都是这么高（以前卡片最高 520，更高的在 iframe 里滚）
  await expect.poll(async () => (await sizes(page)).iframe).toBeGreaterThan(120);
  const size = await sizes(page);
  expect(Math.abs(size.box - size.iframe)).toBeLessThanOrEqual(1);
  expect(size.clipped).toBe(false);
  expect(size.scrolling).toBe("no");
  await expect(card.getByTestId("mcp-app-more")).toHaveCount(0);
  expect(await app.evaluate(() => document.documentElement.scrollHeight - document.documentElement.clientHeight)).toBeLessThanOrEqual(1);
  // 滚轮落在卡片上：滚的是聊天页
  const transcript = page.getByTestId("agent-transcript");
  const iframe = card.getByTestId("mcp-app-iframe");
  await iframe.scrollIntoViewIfNeeded();
  const before = await transcript.evaluate((el) => el.scrollTop);
  const frameBox = (await iframe.boundingBox())!;
  const viewBox = (await transcript.boundingBox())!;
  const y = Math.min(frameBox.y + frameBox.height / 2, viewBox.y + viewBox.height - 40);
  await page.mouse.move(frameBox.x + frameBox.width / 2, Math.max(y, frameBox.y + 20));
  await page.mouse.wheel(0, before > 100 ? -100 : 100);
  await expect.poll(() => transcript.evaluate((el) => el.scrollTop)).not.toBe(before);
});

test("a card taller than the 2000 px guard fades at the bottom and expands in place", async ({ page }) => {
  await open(page);
  const { card, app } = await readyApp(page);
  await app.evaluate(() => {
    const tall = document.createElement("div");
    tall.style.height = "2600px";
    document.body.appendChild(tall);
  });
  const frame = card.locator('[data-testid="mcp-app-frame"]');
  await expect(frame).toHaveAttribute("data-clipped", "true");
  const clipped = await sizes(page);
  expect(clipped.box).toBeLessThanOrEqual(2000);
  expect(clipped.iframe).toBeGreaterThan(2600);
  expect(await frame.evaluate((el) => getComputedStyle(el.firstElementChild as HTMLElement).maskImage)).toContain("linear-gradient");
  expect(await app.evaluate(() => document.documentElement.scrollHeight - document.documentElement.clientHeight)).toBeLessThanOrEqual(1);
  const more = card.getByTestId("mcp-app-more");
  await expect(more).toHaveText("展开");
  await more.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("chat-card-clipped") });
  await more.click();
  await expect(frame).toHaveAttribute("data-clipped", "false");
  const expanded = await sizes(page);
  expect(Math.abs(expanded.box - expanded.iframe)).toBeLessThanOrEqual(1);
  await expect(more).toHaveCount(0);
});
