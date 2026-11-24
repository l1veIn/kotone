import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openSettingsTab } from "./nav";

// 自动卡片和消息提示（ADR 039 D3、D4）：用 @tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://agent-* 事件，不接任何 AI。
// 后端怎么存、什么时候追加给她的那句，由 Rust 单元测试（kotone-agent auto_card）和真机脚本 auto_card_real.py 管。

interface Bridge {
  calls: { command: string; args?: Record<string, unknown> }[];
  event(conversationId: string, event: unknown): Promise<void>;
  focus(conversationId: string, key: string): Promise<void>;
  /** 一张新的自动卡片（这局总结），给“她正在回答时来了卡片”用。 */
  fresh: unknown;
}
declare global {
  interface Window {
    autoCardTest: Bridge;
    messagePopupTest?: { popup: unknown; opened: number[]; closed: number[] };
  }
}

const T = 1_800_000_000_000;
const KEY = "opening:9100000";
/** 截图放在仓库的 target/plugin-shots/round7/（实现记录用）。 */
const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round7/${name}.png`, import.meta.url));

async function open(page: Page) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__auto-card-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__auto-card-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, KEY }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const { sampleMatch } = await import("/src/lib/ipc/lolMock.ts");
    // i18n-ignore-start：示例对话
    const card = {
      role: "notice", text: "英雄联盟 发来一张卡片：开局战绩", atMs: T - 60_000,
      card: { plugin: "lol", pluginName: "英雄联盟", key: KEY, title: "开局战绩", popup: "对面的战绩到了",
        ui: { uri: "ui://kotone/lol/match" }, structuredContent: sampleMatch() },
    };
    const note = {
      role: "user", source: "auto-card", atMs: T - 60_000,
      text: "（系统消息，不是玩家说的话：英雄联盟插件发来一张自动卡片「开局战绩」，玩家在聊天里看得到；要点见附件。）",
      attachments: [{ kind: "data", label: "英雄联盟 · 开局战绩", text: "开局战绩（海克斯大乱斗）。对面可能的突破口：亚索，6 连败。" }],
    };
    const old = {
      version: 1, id: "c-old", title: "上次的对话", createdAtMs: T - 86_400_000, updatedAtMs: T - 60_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        card, note,
      ],
    };
    // i18n-ignore-end
    const conversations: Record<string, typeof old> = { "c-old": old };
    const state = {
      current: "c-old", running: null as string | null, notReadyReason: null,
      conversations: [{ id: "c-old", title: old.title, createdAtMs: old.createdAtMs, updatedAtMs: old.updatedAtMs, messageCount: 4 }],
    };
    const fresh = {
      role: "notice", text: "英雄联盟 发来一张卡片：这局总结", atMs: T + 3_000, // i18n-ignore
      card: { plugin: "lol", pluginName: "英雄联盟", key: "summary:9100000", title: "这局总结", popup: "这局的总结好了", // i18n-ignore
        ui: { uri: "ui://kotone/lol/match" }, structuredContent: sampleMatch() },
    };
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[], fresh,
      async event(conversationId: string, event: unknown) {
        const e = event as { kind: string; message?: unknown };
        if (e.kind === "appended") conversations[conversationId]?.messages.push(e.message as never);
        if (e.kind === "step") state.running = conversationId;
        if (e.kind === "idle") state.running = null;
        await emit("kotone://agent-event", { conversationId, event });
      },
      async focus(conversationId: string, key: string) {
        await emit("kotone://auto-card-focus", { conversationId, key });
      },
    };
    Object.assign(window, { autoCardTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      if (command.startsWith("agent_") || command === "update_settings") bridge.calls.push({ command, args: args as Record<string, unknown> });
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversations[(args as { id: string }).id] ?? null);
        case "agent_held_cards": return [];
        case "automation_runs": return { runs: [] };
        case "usage_conversation": return null;
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((args as { patch: never }).patch);
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

test("an auto card shows in the chat as from 英雄联盟, opens in the right sidebar, and the note for her stays hidden", async ({ page }) => {
  await open(page);
  const card = page.getByTestId("auto-card");
  await expect(card).toHaveCount(1);
  await expect(card.getByTestId("auto-card-from")).toContainText("英雄联盟 发来");
  await expect(card.getByTestId("auto-card-from")).toContainText("开局战绩");
  await expect(card.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]')).toBeVisible();
  // 给她的那句（来源 auto-card）不显示成玩家的气泡；卡片不挂在她的回答下面
  await expect(page.getByTestId("agent-user-message").filter({ hasText: "系统消息" })).toHaveCount(0);
  await expect(page.getByTestId("agent-turn").last().getByTestId("auto-card")).toHaveCount(1);
  await expect(page.getByTestId("agent-turn").last().getByTestId("agent-answer")).toHaveCount(0);
  await card.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("chat-auto-card") });

  await card.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-tab-page")).toHaveAttribute("aria-selected", "true");
  await expect(sidebar.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", "ui://kotone/lol/match");
});

test("a card that comes while she is answering shows at the bottom at once and stays after her answer", async ({ page }) => {
  await open(page);
  await page.evaluate(async (T) => {
    const t = window.autoCardTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "对面谁最菜", source: "voice", atMs: T } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "chunk", text: "我看看" });
    // 她正在回答：卡片先显示（还没记进对话）
    await t.event("c-old", { kind: "autoCard", message: t.fresh });
  }, T);
  const cards = page.getByTestId("auto-card");
  await expect(cards).toHaveCount(2);
  await expect(page.getByTestId("agent-turn").last().getByTestId("auto-card")).toHaveAttribute("data-key", "summary:9100000");
  // 她这一轮还在往下写：流式的字留在她那一轮里，不挂到卡片下面
  await expect(page.locator('[data-turn-active="true"]').getByTestId("agent-answer")).toContainText("我看看");
  await page.evaluate(async (T) => {
    const t = window.autoCardTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "亚索最近 6 连败。", atMs: T + 4_000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
    // 这一轮结束：后端把卡片和给她的那句记进对话（卡片同一条，不重复显示）
    await t.event("c-old", { kind: "appended", message: t.fresh });
    await t.event("c-old", { kind: "appended", message: { role: "user", source: "auto-card", text: "（系统消息……）", atMs: T + 3_000 } });
  }, T);
  await expect(cards).toHaveCount(2);
  const turns = page.getByTestId("agent-turn");
  await expect(turns.last().getByTestId("auto-card")).toHaveAttribute("data-key", "summary:9100000");
  await expect(turns.nth(-2).getByTestId("agent-answer")).toContainText("亚索最近 6 连败。");
  await expect(page.getByTestId("agent-user-message").filter({ hasText: "系统消息" })).toHaveCount(0);
});

test("clicking the message pop-up opens the chat at the card", async ({ page }) => {
  await open(page);
  // 玩家在别的页面（交互设置）
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("assistant-chat")).not.toBeVisible();
  await page.evaluate(async (KEY) => { await window.autoCardTest.focus("c-old", KEY); }, KEY);
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const card = page.locator(`[data-testid="auto-card"][data-key="${KEY}"]`);
  await expect(card).toBeInViewport();
  await expect(card).toHaveClass(/flashing/);
});

test("the switches for auto cards and the message pop-up live in 交互 and default to on", async ({ page }) => {
  await open(page);
  await openSettingsTab(page, "reply");
  const section = page.getByTestId("auto-card-settings");
  await expect(section).toBeVisible();
  await expect(page.getByTestId("auto-cards-enabled")).toBeChecked();
  await expect(page.getByTestId("auto-cards-popup")).toBeChecked();
  await page.locator("#settings-reply-auto-cards").scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("settings-auto-cards") });
  await page.getByTestId("auto-cards-popup").click();
  await expect(page.getByTestId("auto-cards-popup")).not.toBeChecked();
  const patches = await page.evaluate(() => window.autoCardTest.calls.filter((c) => c.command === "update_settings").map((c) => c.args?.patch));
  expect(patches).toContainEqual({ agent: { autoCards: { enabled: true, popup: false } } });
  // 关掉自动卡片：消息提示的开关跟着灰掉
  await page.getByTestId("auto-cards-enabled").click();
  await expect(page.getByTestId("auto-cards-popup")).toBeDisabled();
});

test("the message pop-up window: avatar, name · plugin, one line; click opens, × only closes", async ({ page }) => {
  await page.setViewportSize({ width: 344, height: 104 });
  await page.goto("/#/message-popup");
  const popup = page.getByTestId("message-popup");
  await expect(popup).toBeVisible();
  await expect(page.getByTestId("message-popup-who")).toHaveText("琴音 · 英雄联盟");
  await expect(page.getByTestId("message-popup-text")).toHaveText("对面的战绩到了");
  const box = await popup.boundingBox();
  expect(Math.round(box?.width ?? 0)).toBe(320);
  expect(Math.round(box?.height ?? 0)).toBe(72);
  await page.screenshot({ path: shot("message-popup-preview") });
  // 鼠标停在上面：× 出来；点 × 只关掉
  await popup.hover();
  await page.getByTestId("message-popup-close").click();
  await expect.poll(() => page.evaluate(() => window.messagePopupTest?.closed)).toEqual([1]);
  await expect.poll(() => page.evaluate(() => window.messagePopupTest?.opened)).toEqual([]);
  await expect(popup).toHaveCount(0);
  // 再来一条，点它：打开聊天
  await page.reload();
  await page.getByTestId("message-popup").click();
  await expect.poll(() => page.evaluate(() => window.messagePopupTest?.opened)).toEqual([1]);
});
