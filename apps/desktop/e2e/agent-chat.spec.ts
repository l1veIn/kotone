import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openSettingsTab } from "./nav";

// 聊天页（ADR 028：打开 Kotone 先看到它）：用 @tauri-apps/api/mocks 模拟 agent_* 命令与 kotone://agent-* 事件，不接任何 AI。

interface AgentCall { command: string; args?: Record<string, unknown> }
declare global {
  interface Window {
    agentTest: {
      calls: AgentCall[];
      state: { current: string | null; running: string | null; notReadyReason: string | null; conversations: { id: string; title: string; createdAtMs: number; updatedAtMs: number; messageCount: number }[] };
      conversations: Record<string, { version: number; id: string; title: string; createdAtMs: number; updatedAtMs: number; messages: unknown[] }>;
      event(conversationId: string, event: unknown): Promise<void>;
      ask(askId: string, conversationId: string, callId: string): Promise<void>;
      resolved(askId: string, conversationId: string): Promise<void>;
    };
  }
}

const T = 1_800_000_000_000;

async function openChat(page: Page, options: { empty?: boolean; notReady?: string; asr?: "online" | "local-stopped" } = {}) {
  await page.route("**/__agent-chat-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__agent-chat-test#/settings?onboarding=never");
  await page.evaluate(async ({ empty, notReady, asr, T }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const old = {
      version: 1, id: "c-old", title: "上次的对话", createdAtMs: T - 86_400_000, updatedAtMs: T - 86_000_000,
      messages: [
        { role: "user", text: "上次的问题", source: "voice", atMs: T - 86_400_000 },
        { role: "assistant", text: "上次的回答", atMs: T - 86_399_000 },
      ],
    };
    const conversations: Record<string, typeof old> = { "c-old": old };
    const state = {
      current: empty ? null : "c-old", running: null as string | null, notReadyReason: notReady ?? null,
      conversations: [{ id: "c-old", title: old.title, createdAtMs: old.createdAtMs, updatedAtMs: old.updatedAtMs, messageCount: 2 }],
    };
    let created = 0;
    // 语音识别：默认在线、已就绪；local-stopped = 本地识别还没启动（ADR 028）
    const runtime = asr === "local-stopped"
      ? { phase: "stopped", restartNeeded: false, engineId: "sherpa-streaming", engineName: "本地", modelId: "x-asr", interactionMode: "dictation", stage: null, asrLocal: true, error: null }
      : { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null, asrLocal: false, error: null };
    let connectionModel = "test";
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[], state, conversations,
      async event(conversationId: string, event: unknown) {
        // 与后端一致：每条 appended 的消息同时存盘
        const e = event as { kind: string; message?: unknown };
        if (e.kind === "appended") conversations[conversationId]?.messages.push(e.message as never);
        if (e.kind === "compacted") conversations[conversationId]?.messages.splice((e as unknown as { index: number }).index, 0, e.message as never);
        if (e.kind === "step") state.running = conversationId;
        if (e.kind === "idle") state.running = null;
        await emit("kotone://agent-event", { conversationId, event });
      },
      async ask(askId: string, conversationId: string, callId: string) {
        await emit("kotone://agent-ask", { askId, conversationId, request: { callId, tool: "game_chat.send", title: "发到游戏聊天", reason: "会在游戏里发言", arguments: { text: "gg" } } });
      },
      async resolved(askId: string, conversationId: string) {
        await emit("kotone://agent-ask-resolved", { askId, conversationId, answer: "deny" });
      },
    };
    Object.assign(window, { agentTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      if (command.startsWith("agent_") || command === "ui_open_reply") bridge.calls.push({ command, args: args as Record<string, unknown> });
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversations[(args as { id: string }).id] ?? null);
        case "agent_send": {
          if (state.running) return { status: "steered", conversationId: state.running };
          if (!state.current) {
            const id = `c-new-${++created}`;
            conversations[id] = { version: 1, id, title: String((args as { text: string }).text), createdAtMs: T, updatedAtMs: T, messages: [] };
            state.conversations.unshift({ id, title: conversations[id].title, createdAtMs: T, updatedAtMs: T, messageCount: 0 });
            state.current = id;
          }
          return { status: "started", conversationId: state.current };
        }
        case "agent_stop": return true;
        case "agent_set_permissions": return (args as { permissions: unknown }).permissions;
        case "agent_tools": return { tools: [
          { name: "game_chat.send", group: "game", description: "把一句话打进游戏聊天框并发出", sideEffects: ["input"], defaultRule: "ask", rule: null, core: true },
          { name: "reply.say", group: "game", description: "用语音或弹幕告诉玩家一句话", sideEffects: [], defaultRule: "allow", rule: null, core: true },
          { name: "window.list", group: "screen", description: "列出窗口", sideEffects: [], defaultRule: "allow", rule: null, core: false },
          { name: "shell.run", group: "computer", description: "在玩家电脑上执行 PowerShell 命令（只在后台）", sideEffects: ["runs-command"], defaultRule: "allow", rule: null, core: false },
        ] };
        case "agent_set_automation_hidden": return (args as { hidden: boolean; id: string }).hidden ? [(args as { id: string }).id] : [];
        case "agent_answer": return true;
        case "agent_new_conversation": state.current = null; return null;
        case "agent_select_conversation": state.current = (args as { id: string }).id; return null;
        case "agent_delete_conversation": {
          const id = (args as { id: string }).id;
          delete conversations[id];
          state.conversations = state.conversations.filter((c) => c.id !== id);
          if (state.current === id) state.current = null;
          return true;
        }
        case "automation_runs": return { runs: [] };
        case "usage_summary": {
          const today = { requests: 9, inputTokens: 182_000, cacheHitTokens: 179_000, outputTokens: 1_200, characters: 300, audioMs: 65_000, images: 2, hitRate: 179_000 / 182_000, cost: { CNY: 0.0251 }, unpriced: 0 };
          return { today, month: { ...today, cost: { CNY: 1.204 }, unpriced: 3 }, byKind: [{ kind: "chat", label: "对话", totals: today }, { kind: "compaction", label: "压缩", totals: { ...today, requests: 1 } }],
            byModel: [], days: [
              { day: "2026-11-17", totals: { ...today, requests: 4, cost: { CNY: 0.012 } }, byKind: [{ kind: "chat", label: "对话", totals: today }], byModel: [{ model: "deepseek/deepseek-flash", totals: today }] },
              { day: "2026-11-18", totals: today, byKind: [{ kind: "chat", label: "对话", totals: today }, { kind: "tts", label: "语音合成", totals: { ...today, requests: 2, inputTokens: 0, cacheHitTokens: 0, cost: { CNY: 0.003 } } }], byModel: [{ model: "deepseek/deepseek-flash", totals: today }] },
            ], priceSource: "DeepSeek 官方价格页（测试）", note: "费用是估算", ledger: "C:/test/usage.jsonl" };
        }
        case "usage_conversation": {
          // 账本里记到这段对话的语音、读屏费用（模型的费用在消息里）
          const id = (args as { id: string }).id;
          const voice = { requests: 2, inputTokens: 0, cacheHitTokens: 0, outputTokens: 0, characters: 60, audioMs: 4_000, images: 0, hitRate: null, cost: { CNY: 0.005 }, unpriced: 0 };
          const screen = { ...voice, characters: 0, audioMs: 0, images: 1, requests: 1, cost: { CNY: 0.0015 } };
          const search = { ...voice, characters: 0, audioMs: 0, requests: 1, cost: { CNY: 0.0055 } };
          const turn = (window as unknown as { costTurn?: number }).costTurn;
          if (id !== "c-old" || !turn) return null;
          // 交给后台的事的模型费用（ADR 029 第二期）：不在这段对话的消息里，按“后台”记到派它的那一轮
          if ((window as unknown as { costBackground?: boolean }).costBackground) {
            const background = { ...voice, characters: 0, audioMs: 0, requests: 3, inputTokens: 9_000, cacheHitTokens: 8_000, cost: { CNY: 0.003 } };
            return { conversationId: id,
              total: [{ category: "voice", label: "语音", totals: voice }, { category: "background", label: "后台", totals: background }],
              turns: [{ turn, byCategory: [{ category: "voice", label: "语音", totals: voice }, { category: "background", label: "后台", totals: background }] }] };
          }
          return { conversationId: id,
            total: [{ category: "voice", label: "语音", totals: voice }, { category: "screen", label: "读屏", totals: screen }, { category: "search", label: "搜索", totals: search }],
            turns: [{ turn, byCategory: [{ category: "voice", label: "语音", totals: voice }, { category: "screen", label: "读屏", totals: screen }] }] };
        }
        case "get_runtime_status": return structuredClone(runtime);
        case "start_runtime": {
          bridge.calls.push({ command });
          runtime.phase = "running";
          await emit("kotone://runtime", runtime);
          return structuredClone(runtime);
        }
        case "assistant_call": bridge.calls.push({ command }); return null;
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: connectionModel, hasApiKey: true }];
        case "list_connection_presets": return [{ id: "test", displayName: "测试", group: "custom", description: "", defaultBaseUrl: "https://example.invalid", defaultModel: "test", models: ["test", "test-pro"] }];
        case "upsert_connection": {
          bridge.calls.push({ command, args: args as Record<string, unknown> });
          connectionModel = String((args as { connection: { model: string } }).connection.model);
          return browser.getSettings();
        }
        case "connection_models":
          bridge.calls.push({ command, args: args as Record<string, unknown> });
          return { supported: true, models: ["test", "test-pro", "test-listed"] };
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((args as { patch: never }).patch);
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { empty: options.empty ?? false, notReady: options.notReady, asr: options.asr ?? "online", T });
  // 打开就是聊天页（ADR 028），不用再点侧栏。
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const agentCalls = (page: Page, command: string) =>
  page.evaluate((command) => window.agentTest.calls.filter((call) => call.command === command).map((call) => call.args ?? null), command);

test("empty conversation shows starter prompts; a starter sends or fills the composer", async ({ page }, testInfo) => {
  await openChat(page, { empty: true });
  const starters = page.getByTestId("agent-starter");
  await expect(starters).toHaveText(["你能做什么？", "帮我配置 Kotone", "这个版本什么英雄强？", "来首歌", "做一个自动化", "5 分钟后提醒我"]);
  await page.screenshot({ path: testInfo.outputPath("agent-chat-empty.png"), animations: "disabled" });
  // 以冒号结尾的提示只放进输入框
  await starters.filter({ hasText: "做一个自动化" }).click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("帮我做一个自动化：");
  expect(await agentCalls(page, "agent_send")).toEqual([]);
  // 其它提示直接发出，并切到新建的对话
  await starters.filter({ hasText: "你能做什么？" }).click();
  expect(await agentCalls(page, "agent_send")).toEqual([{ text: "你能做什么？" }]);
  await expect(page.getByTestId("agent-conversation-title")).toHaveText(/你能做什么？/);
});

test("streams text and thinking, tracks tool card states, asks for permission and shows usage", async ({ page }, testInfo) => {
  await openChat(page);
  const composer = page.getByTestId("assistant-request");
  await composer.fill("帮我跟队友说 gg");
  await composer.press("Enter");
  expect(await agentCalls(page, "agent_send")).toEqual([{ text: "帮我跟队友说 gg" }]);
  await expect(composer).toHaveValue("");

  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "帮我跟队友说 gg", source: "chat", atMs: T } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "thought", text: "用户想在游戏里发 gg，" });
    await t.event("c-old", { kind: "thought", text: "需要 game_chat.send。" });
    await t.event("c-old", { kind: "chunk", text: "好的，" });
    await t.event("c-old", { kind: "chunk", text: "我来**发**。" });
  }, T);
  await expect(page.getByTestId("agent-stop")).toBeVisible();
  await expect(page.getByTestId("agent-send")).toHaveText("插话");
  const live = page.locator('[data-testid="agent-answer"][data-streaming="true"]');
  await expect(live).toHaveText("好的，我来发。");
  await expect(live.locator("strong")).toHaveText("发");
  await expect(page.locator('[data-testid="agent-thinking"][data-streaming="true"]')).toContainText("用户想在游戏里发 gg，需要 game_chat.send。");

  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "好的，我来**发**。", reasoning: "用户想在游戏里发 gg，需要 game_chat.send。", toolCalls: [{ id: "call-1", name: "game_chat.send", arguments: '{"text":"gg"}' }, { id: "call-2", name: "saved.abc", arguments: "{}" }], usage: { inputTokens: 1200, outputTokens: 30 }, atMs: T + 1000 } });
    await t.event("c-old", { kind: "toolCall", call: { id: "call-1", name: "game_chat.send", arguments: '{"text":"gg"}' } });
    await t.event("c-old", { kind: "toolCall", call: { id: "call-2", name: "saved.abc", arguments: "{}" } });
  }, T);
  await expect(page.locator('[data-testid="agent-answer"][data-streaming="true"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="agent-thinking"]').last()).toHaveAttribute("data-streaming", "false");
  const send = page.locator('[data-tool-id="call-1"]');
  const saved = page.locator('[data-tool-id="call-2"]');
  await expect(send).toHaveAttribute("data-tool-status", "pending");
  await expect(send).toContainText("发到游戏聊天");
  await expect(send.getByTestId("agent-tool-status")).toHaveText("等待中");
  await expect(saved).toContainText("运行已保存的自动化");

  await page.evaluate(async () => {
    await window.agentTest.event("c-old", { kind: "toolCallUpdate", callId: "call-1", status: "in_progress", content: null });
    await window.agentTest.ask("ask-1", "c-old", "call-1");
  });
  await expect(send).toHaveAttribute("data-tool-status", "in_progress");
  await expect(send.getByTestId("agent-tool-status")).toHaveText("执行中");
  await expect(send.getByTestId("agent-ask")).toContainText("需要你确认：发到游戏聊天");
  await page.screenshot({ path: testInfo.outputPath("agent-chat-ask.png"), animations: "disabled" });
  await send.getByTestId("agent-ask-allow-conversation").click();
  expect(await agentCalls(page, "agent_answer")).toEqual([{ askId: "ask-1", answer: "allow-for-conversation" }]);
  await page.evaluate(() => window.agentTest.resolved("ask-1", "c-old"));
  await expect(send.getByTestId("agent-ask")).toHaveCount(0);

  // 参数与结果默认折叠
  await expect(send.getByText('"text": "gg"')).toHaveCount(0);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "toolCallUpdate", callId: "call-1", status: "completed", content: '{"sent":true}' });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-1", name: "game_chat.send", content: '{"sent":true}', isError: false, untrusted: false, atMs: T + 2000 } });
    await t.event("c-old", { kind: "toolCallUpdate", callId: "call-2", status: "failed", content: "没有这个自动化" });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-2", name: "saved.abc", content: "没有这个自动化", isError: true, untrusted: false, atMs: T + 2100 } });
  }, T);
  await expect(send).toHaveAttribute("data-tool-status", "completed");
  await expect(saved).toHaveAttribute("data-tool-status", "failed");
  await expect(saved.getByTestId("agent-tool-status")).toHaveText("失败");
  await send.locator("summary").click();
  await expect(send.getByText('"text": "gg"')).toBeVisible();
  await expect(send.getByText('"sent": true')).toBeVisible();

  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "step", step: 2 });
    await t.event("c-old", { kind: "chunk", text: "已经发了。" });
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "已经发了。", usage: { inputTokens: 1400, outputTokens: 8, cachedInputTokens: 1000, cost: 0.0012, currency: "CNY" }, atMs: T + 3000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  await expect(page.getByTestId("agent-stop")).toHaveCount(0);
  await expect(page.getByTestId("agent-send")).toHaveText("发送");
  const footer = page.getByTestId("agent-turn").last().getByTestId("agent-turn-usage");
  // 命中率 = 命中 ÷ 输入（照 DeepSeek Harness）；费用按价格表（DeepSeek 用人民币）。
  await expect(footer).toHaveText("输入 2,600（命中缓存 38%） · 输出 38 tokens · 约 ¥0.0012");
  // 旧的一轮没有用量：不显示、不估算
  await expect(page.getByTestId("agent-turn").first().getByTestId("agent-turn-usage")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("agent-chat-done.png"), animations: "disabled" });
});

test("stop, steering while answering, notices and automation results", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "游戏里说的话", source: "voice", atMs: T } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "chunk", text: "我看看" });
  }, T);
  await expect(page.getByTestId("agent-user-source").last()).toHaveText("通话");
  // 回答中仍可发送：算插话
  const composer = page.getByTestId("assistant-request");
  await composer.fill("顺便看看队友");
  await composer.press("Enter");
  expect(await agentCalls(page, "agent_send")).toEqual([{ text: "顺便看看队友" }]);
  await expect(page.getByTestId("agent-send-notice")).toContainText("已插话");
  // Shift+Enter 换行，不发送
  await composer.fill("第一行");
  await composer.press("Shift+Enter");
  await composer.pressSequentially("第二行");
  await expect(composer).toHaveValue("第一行\n第二行");
  expect(await agentCalls(page, "agent_send")).toHaveLength(1);

  await page.getByTestId("agent-stop").click();
  expect(await agentCalls(page, "agent_stop")).toHaveLength(1);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "我看看", atMs: T + 500 } });
    await t.event("c-old", { kind: "appended", message: { role: "notice", text: "已停止", atMs: T + 600 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "cancelled" }, acpStopReason: "cancelled" });
    await t.event("c-old", { kind: "idle" });
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "（系统消息，不是玩家说的话：自动化「连按Q」已完成。执行记录见附件；需要的话告诉玩家结果。）", attachments: [{ kind: "data", label: "执行记录", text: '{"id":"run-7","status":"finished"}' }], source: "automation", atMs: T + 5000 } });
  }, T);
  await expect(page.getByTestId("agent-notice").last()).toHaveText("已停止");
  await expect(page.getByTestId("agent-stop")).toHaveCount(0);
  await expect(page.getByTestId("agent-system-message")).toContainText("自动化「连按Q」已完成");
  await expect(page.getByTestId("agent-system-message")).not.toContainText("系统消息");
  // 自动化结果是系统卡片，不是用户气泡
  await expect(page.getByTestId("agent-user-message").filter({ hasText: "连按Q" })).toHaveCount(0);
});

test("IME composition Enter never sends", async ({ page }) => {
  await openChat(page);
  const composer = page.getByTestId("assistant-request");
  await composer.fill("nihao");
  await composer.dispatchEvent("compositionstart");
  await composer.press("Enter");
  await composer.dispatchEvent("compositionend");
  await composer.evaluate((element) => element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true })));
  expect(await agentCalls(page, "agent_send")).toEqual([]);
});

test("conversation list: new, switch and delete", async ({ page }, testInfo) => {
  await openChat(page);
  const list = page.getByTestId("agent-conversations");
  await expect(list.getByTestId("agent-conversation")).toHaveCount(1);
  await expect(page.getByTestId("agent-user-message").first()).toHaveText("上次的问题");

  await list.getByTestId("agent-new-conversation").click();
  expect(await agentCalls(page, "agent_new_conversation")).toHaveLength(1);
  await expect(page.getByTestId("agent-empty")).toBeVisible();
  await expect(list.getByTestId("agent-conversation-new")).toBeVisible();

  await list.getByTestId("agent-conversation").first().click();
  expect(await agentCalls(page, "agent_select_conversation")).toEqual([{ id: "c-old" }]);
  await expect(page.getByTestId("agent-user-message").first()).toHaveText("上次的问题");

  await list.getByTestId("agent-conversation").first().hover();
  await list.getByTestId("agent-conversation-delete").first().click();
  await expect(page.getByTestId("agent-delete-dialog")).toContainText("上次的对话");
  await page.screenshot({ path: testInfo.outputPath("agent-chat-delete.png"), animations: "disabled" });
  await page.getByTestId("agent-delete-confirm").click();
  expect(await agentCalls(page, "agent_delete_conversation")).toEqual([{ id: "c-old" }]);
  await expect(list.getByTestId("agent-conversation")).toHaveCount(0);
  await expect(page.getByTestId("agent-empty")).toBeVisible();
});

test("switching and deleting are disabled while answering", async ({ page }) => {
  await openChat(page);
  await page.evaluate(() => window.agentTest.event("c-old", { kind: "step", step: 1 }));
  const list = page.getByTestId("agent-conversations");
  await expect(list.getByTestId("agent-new-conversation")).toBeDisabled();
  await expect(list.getByTestId("agent-conversation-delete")).toBeHidden();
});

test("not ready shows the reason and opens the model menu next to the chat box", async ({ page }) => {
  await openChat(page, { notReady: "请在聊天页输入框旁的「她用的模型」里选一个" });
  await expect(page.getByTestId("agent-not-ready")).toContainText("请在聊天页输入框旁的「她用的模型」里选一个");
  await page.getByTestId("assistant-request").fill("你好");
  await expect(page.getByTestId("agent-send")).toBeDisabled();
  // 选模型回到了输入框旁边（交接第十一节第 8 条）：「换个模型」直接打开那个菜单，不再跳去设置
  await expect(page.getByTestId("agent-open-settings")).toHaveText("换个模型 →");
  await page.getByTestId("agent-open-settings").click();
  await expect(page.getByTestId("composer-model-menu")).toBeVisible();
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
});

test("agent settings replace permissions as a whole and save the in-game switch", async ({ page }) => {
  await openChat(page);
  // 权限在角色页（原「设置」的陪玩组，交接第十一节第 8 条）
  await openSettingsTab(page, "permissions");
  const settings = page.getByTestId("agent-settings");
  await settings.getByTestId("agent-mode-read-only").click();
  await expect(settings.getByTestId("agent-mode-read-only")).toHaveAttribute("aria-pressed", "true");
  await settings.getByTestId("agent-rules").locator("summary").click();
  // 工具清单来自 agent_tools：按分组列出，显示默认规则
  await expect(settings.getByTestId("agent-rule")).toHaveCount(4);
  await expect(settings.locator('[data-tool="game_chat.send"]').getByTestId("agent-rule-default")).toHaveText("默认：询问");
  await expect(settings.getByRole("heading", { name: "屏幕与窗口", exact: true })).toBeVisible();
  await settings.getByLabel("发到游戏聊天 的规则").click();
  await page.getByRole("option", { name: "禁止" }).click();
  await expect(settings.getByLabel("发到游戏聊天 的规则")).toHaveText("禁止");
  await settings.getByLabel("发到游戏聊天 的规则").click();
  await page.getByRole("option", { name: "按默认" }).click();
  // 后台执行命令、读写文件（ADR 030）在“电脑”一组，默认允许（什么时候问由规则定），玩家可以禁止
  await expect(settings.getByRole("heading", { name: "电脑（命令和文件）", exact: true })).toBeVisible();
  await expect(settings.locator('[data-tool="shell.run"]').getByTestId("agent-rule-default")).toHaveText("默认：允许");
  await settings.getByLabel("运行命令 的规则").click();
  await page.getByRole("option", { name: "禁止" }).click();
  // 每次都整体替换：删掉的规则不会留在补丁里
  expect(await agentCalls(page, "agent_set_permissions")).toEqual([
    { permissions: { mode: "read-only", rules: {} } },
    { permissions: { mode: "read-only", rules: { "game_chat.send": "deny" } } },
    { permissions: { mode: "read-only", rules: {} } },
    { permissions: { mode: "read-only", rules: { "shell.run": "deny" } } },
  ]);
  await settings.getByTestId("agent-in-game").click();
  await expect(settings.getByTestId("agent-in-game")).toHaveAttribute("aria-checked", "false");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
  expect(saved.agent?.inGame).toBe(false);
});

test("composer bar: usage inside; her model, permission and call sit next to send", async ({ page }) => {
  await openChat(page);
  const composer = page.getByTestId("agent-composer");
  // 用量在输入框里面（rc2 玩家反馈：计费挪进 chatbox）
  await expect(composer.getByTestId("usage-pill")).toBeVisible();
  // 她用的模型回到了输入框旁边（2026-11-25 玩家：“模型选择这块，我觉得还是放在 chat 聊天吧”）
  const model = composer.getByTestId("composer-model");
  await expect(model).toHaveText("test");
  await expect(model).toHaveAttribute("title", "她用的模型：test");
  // 呼叫：发送键旁边的一个图标，悬停看快捷键
  const call = composer.getByTestId("call-kotone");
  await expect(call).toHaveAttribute("title", /呼叫琴音（呼叫键 Ctrl \+ ~）/);
  await expect(call.locator("svg")).toHaveCount(1);
  const callBox = (await call.boundingBox())!;
  const sendBox = (await composer.getByTestId("agent-send").boundingBox())!;
  expect(sendBox.x - (callBox.x + callBox.width)).toBeLessThan(16);
  // 权限：一个盾牌图标，悬停看当前模式，点开照旧换模式
  const permission = composer.getByTestId("composer-permission");
  await expect(permission).toHaveAttribute("title", "权限：先问我");
  expect((await permission.boundingBox())!.width).toBeLessThan(40);
  await page.screenshot({ path: fileURLToPath(new URL("../../../target/plugin-shots/round7/chat-composer.png", import.meta.url)), animations: "disabled" });
  await permission.click();
  await page.getByTestId("composer-permission-allow-all").click();
  await expect(permission).toHaveAttribute("title", "权限：完全权限");
  expect(await agentCalls(page, "agent_set_permissions")).toEqual([{ permissions: { mode: "allow-all", rules: {} } }]);

  // 选模型：清单来自服务商的模型列表（照 DeepSeek Harness），不是写死的预置名
  await model.click();
  const menu = page.getByTestId("composer-model-menu");
  await expect(menu).toContainText("她用的模型");
  await expect(menu.getByTestId("composer-model-test-ai-test-listed")).toBeVisible();
  await expect(menu.getByTestId("composer-model-test-ai-test")).toContainText("test");
  // 截图放在仓库的 target/plugin-shots/round7/（设置重排，交接第十一节第 8 条）
  await page.screenshot({ path: fileURLToPath(new URL("../../../target/plugin-shots/round7/chat-model-menu.png", import.meta.url)), animations: "disabled" });
  await menu.getByTestId("composer-model-test-ai-test-pro").click();
  await expect(model).toHaveText("test-pro");
  const calls = await page.evaluate(() => window.agentTest.calls.filter((c) => c.command === "upsert_connection" || c.command === "connection_self_check"));
  // 换模型不自检；改的是她用的连接的模型（和原来「设置 → 她怎么回答 → 她用的模型」同一个字段）
  expect(calls.map((c) => c.command)).toEqual(["upsert_connection"]);
  expect((calls[0].args as { connection: { id: string; model: string } }).connection).toMatchObject({ id: "test-ai", model: "test-pro" });
  // 「交互」里不再有选模型，只留一行指回聊天页
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("settings-reply").getByTestId("assistant-model")).toHaveCount(0);
  await expect(page.getByTestId("settings-reply").getByLabel("她用的 API 连接")).toHaveCount(0);
  await expect(page.getByTestId("composer-model")).toBeHidden();
  await page.getByTestId("settings-reply-open-model").click();
  await expect(page.getByTestId("composer-model-menu")).toBeVisible();
});

test("tool cards link the settings 琴音 changed, and a key request is pasted into a private box", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "", toolCalls: [
      { id: "call-s", name: "settings.update", arguments: '{"patch":{"assistant":{"hotkey":"F9"}}}' },
      { id: "call-m", name: "models.activate", arguments: '{"category":"asr","modelId":"deepseek-asr"}' },
      { id: "call-l", name: "models.list", arguments: "{}" },
    ], atMs: T + 1000 } });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-s", name: "settings.update", isError: false, untrusted: false, atMs: T + 1100,
      content: JSON.stringify({ effects: { changed: ["assistant.hotkey"], locations: [{ path: "assistant.hotkey", label: "呼叫键", page: "设置 → 怎么叫她 → 呼叫键" }], summary: "已保存，立即生效" } }) } });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-m", name: "models.activate", content: "{}", isError: false, untrusted: false, atMs: T + 1200 } });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-l", name: "models.list", content: "{}", isError: false, untrusted: false, atMs: T + 1300 } });
  }, T);
  // 只读的工具没有链接
  await expect(page.locator('[data-tool-id="call-l"]').getByTestId("agent-tool-changed")).toHaveCount(0);
  await expect(page.locator('[data-tool-id="call-m"]').getByTestId("agent-tool-changed-link")).toHaveText("语音识别模型 →");
  const link = page.locator('[data-tool-id="call-s"]').getByTestId("agent-tool-changed-link");
  await expect(link).toHaveText("呼叫键 →");
  await link.click();
  await expect(page.getByTestId("assistant-nav-call")).toHaveAttribute("aria-current", "page");
  await expect(page.locator("#settings-call-hotkey")).toBeInViewport();
  // 回到聊天：侧栏的对话列表
  await page.getByTestId("sidebar-conversations").getByText("上次的对话").click();
  await page.locator('[data-tool-id="call-m"]').getByTestId("agent-tool-changed-link").click();
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");

  // 琴音要别的服务商的 Key：聊天里出现只给玩家看的输入框
  await page.getByTestId("sidebar-conversations").getByText("上次的对话").click();
  await page.evaluate(async () => {
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("kotone://agent-key-request", { requestId: "key-1", conversationId: "c-old", provider: "openai", displayName: "OpenAI", apiKeyUrl: "https://platform.openai.com/api-keys", connectionId: "", reason: "在线语音合成要用" });
  });
  const card = page.getByTestId("agent-key-request");
  await expect(card).toContainText("琴音需要 OpenAI 的 Key");
  await expect(card).toContainText("不会出现在聊天里");
  await expect(page.getByTestId("agent-key-input")).toHaveAttribute("type", "password");
  await page.getByTestId("agent-key-input").fill("sk-test-123");
  await page.getByTestId("agent-key-save").click();
  expect(await agentCalls(page, "agent_submit_key")).toEqual([{ requestId: "key-1", provider: "openai", connectionId: null, apiKey: "sk-test-123" }]);
  await page.evaluate(async () => {
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("kotone://agent-key-request-resolved", { requestId: "key-1", conversationId: "c-old", saved: true });
  });
  await expect(card).toHaveCount(0);
  // Key 不会出现在聊天记录里
  await expect(page.getByTestId("assistant-chat")).not.toContainText("sk-test-123");
});

test("compaction divider, match recap card and switch; usage shows hit rate and cost", async ({ page }, testInfo) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "compacted", index: 1, message: { role: "notice", text: "较早的对话已整理成摘要（之后发给 AI 的是摘要和最近的几轮）", atMs: T, compaction: "## 玩家的要求和意图\n- 记住召唤师名字“星河下的猫”", usage: { inputTokens: 25_000, cachedInputTokens: 24_000, outputTokens: 1_000, cost: 0.012, currency: "CNY" } } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  const checkpoint = page.getByTestId("agent-checkpoint");
  await expect(checkpoint).toHaveCount(1);
  await expect(checkpoint).toContainText("较早的对话已整理成摘要");
  await expect(checkpoint.getByText("星河下的猫")).toBeHidden();
  await checkpoint.locator("summary").click();
  await expect(checkpoint.getByText("星河下的猫")).toBeVisible();

  // 对局复盘：游戏退出后 Kotone 留在聊天里的一条（只给玩家看）
  await page.evaluate(async (T) => {
    await window.agentTest.event("c-old", { kind: "appended", message: { role: "notice", text: "对局复盘：测试游戏（32 分钟），已记进这个游戏的记忆", atMs: T + 9000, recap: "- 拿了两条龙\n- 你让我提醒买真眼" } });
  }, T);
  const recap = page.getByTestId("agent-recap");
  await expect(recap).toContainText("对局复盘：测试游戏");
  await expect(recap).toContainText("你让我提醒买真眼");

  // 权限、用量在角色页（交接第十一节第 8 条）
  await openSettingsTab(page, "permissions");
  // 对局复盘可以关
  await page.getByTestId("agent-match-recap").click();
  await expect.poll(async () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings().then((s) => s.agent?.matchRecap))).toBe(false);
  // DeepSeek 内容审核（默认开）可以关；思考强度默认跟服务商
  await page.getByTestId("agent-safety-check").click();
  await expect.poll(async () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings().then((s) => s.agent?.safetyCheck))).toBe(false);
  await expect(page.getByRole("combobox", { name: "通话里的思考强度" })).toContainText("默认");
  await expect(page.getByRole("combobox", { name: "聊天窗口里的思考强度" })).toContainText("默认");
  await page.getByTestId("assistant-nav-usage").click();
  const usage = page.getByTestId("usage-settings");
  await expect(usage.getByTestId("usage-today")).toContainText("¥0.025");
  // 没全中不显示 100%（照 DeepSeek Harness）：179000 / 182000 = 98.4%
  await expect(usage.getByTestId("usage-today-hit")).toHaveText("98%");
  await expect(usage.getByTestId("usage-month")).toContainText("3 次没有价格");
  await expect(usage.getByTestId("usage-by-kind")).toContainText("压缩");
  await usage.getByLabel("整理阈值（token）").fill("32000");
  await usage.getByLabel("整理阈值（token）").press("Tab");
  await expect.poll(async () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings().then((s) => s.agent?.compactAtTokens))).toBe(32000);
  await usage.screenshot({ path: testInfo.outputPath("usage.png"), animations: "disabled" });
});

test("the chat shows today's and this conversation's usage; details by day and by call", async ({ page }, testInfo) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "好", usage: { inputTokens: 20_000, cachedInputTokens: 19_800, outputTokens: 50, cost: 0.0015, currency: "CNY" }, atMs: T + 1000 } });
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "嗯", usage: { inputTokens: 18_000, cachedInputTokens: 100, outputTokens: 40, cost: 0.036, currency: "CNY" }, atMs: T + 2000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  const pill = page.getByTestId("usage-pill");
  await expect(page.getByTestId("usage-pill-today")).toHaveText("今天 ¥0.025 · 命中 98%");
  // 本段对话：(19800 + 100) / 38000 = 52%
  await expect(page.getByTestId("usage-pill-conversation")).toHaveText("本段对话 ¥0.037 · 命中 52%");
  await pill.click();
  const dialog = page.getByTestId("usage-dialog");
  await expect(dialog.getByTestId("usage-day")).toHaveCount(2);
  await expect(dialog.getByTestId("usage-day").first()).toHaveAttribute("data-day", "2026-11-18");
  await dialog.getByTestId("usage-day").first().click();
  await expect(dialog.getByTestId("usage-day-detail")).toContainText("语音合成");
  await expect(dialog.getByTestId("usage-day-detail")).toContainText("deepseek/deepseek-flash");
  await page.screenshot({ path: testInfo.outputPath("usage-days.png"), animations: "disabled" });
  await dialog.getByTestId("usage-tab-conversation").click();
  await expect(dialog.getByTestId("usage-call")).toHaveCount(2);
  // 新的在前；没命中的那次标出来
  await expect(dialog.getByTestId("usage-call").first()).toContainText("1%");
  await expect(dialog.getByTestId("usage-call").first().locator("td.text-warning")).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath("usage-calls.png"), animations: "disabled" });
});

test("web search lists its sources under the tool card", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "", toolCalls: [
      { id: "call-w", name: "web.search", arguments: '{"query":"海克斯大乱斗 强化 推荐"}' },
    ], atMs: T + 1000 } });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "call-w", name: "web.search", isError: false, untrusted: true, atMs: T + 1100,
      content: JSON.stringify({ results: [
        { title: "海克斯大乱斗强化推荐", url: "https://example.com/aram", site: "九游", date: "2026-11-14", summary: "…" },
        { title: "", url: "https://example.com/b", summary: "…" },
        { title: "坏链接", url: "javascript:alert(1)", summary: "…" },
      ] }) } });
  }, T);
  const card = page.locator('[data-tool-id="call-w"]');
  await expect(card.getByTestId("agent-tool-status")).toContainText("完成");
  await expect(card).toContainText("联网搜索");
  const sources = card.getByTestId("agent-tool-source");
  await expect(sources).toHaveCount(2);
  await expect(card.getByTestId("agent-tool-sources")).toContainText("九游 · 11-14");
  await expect(sources.nth(1)).toHaveText("https://example.com/b");
});

test("local recognition not started: the chat says so, and the call button starts it before listening", async ({ page }, testInfo) => {
  await openChat(page, { empty: true, asr: "local-stopped" });
  await expect(page.getByTestId("chat-local-asr")).toContainText("本地语音识别 · 未启动");
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-stopped");
  await expect(page.getByTestId("chat-call-hint")).toContainText("点一下会先启动本地识别");
  await page.screenshot({ path: testInfo.outputPath("chat-local-stopped.png"), animations: "disabled" });
  await page.getByTestId("chat-call").click();
  await expect.poll(() => page.evaluate(() => window.agentTest.calls
    .filter((call) => call.command === "start_runtime" || call.command === "assistant_call").map((call) => call.command)))
    .toEqual(["start_runtime", "assistant_call"]);
  await expect(page.getByTestId("chat-local-asr")).toHaveCount(0);
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-running");
});

test("online recognition: the call button listens right away and says how else to call her", async ({ page }) => {
  await openChat(page, { empty: true });
  await expect(page.getByTestId("chat-local-asr")).toHaveCount(0);
  await expect(page.getByTestId("titlebar-asr")).toHaveCount(0);
  await expect(page.getByTestId("chat-call-hint")).toContainText("或在游戏里按 Ctrl + ~");
  await page.getByTestId("chat-call").click();
  await expect.poll(() => page.evaluate(() => window.agentTest.calls
    .filter((call) => call.command === "start_runtime" || call.command === "assistant_call").map((call) => call.command)))
    .toEqual(["assistant_call"]);
});

test("after the AI connection changes, the chat re-reads her state instead of keeping a stale 还没准备好", async ({ page }) => {
  // 2026-11-20 真机：填完 Key 进聊天，聊天页还停在打开软件时读到的“还没有选 AI 连接”，发不出去。
  await openChat(page, { notReady: "还没有选 API 连接：请在聊天页输入框旁的「她用的模型」里选一个" });
  await expect(page.getByTestId("agent-not-ready")).toBeVisible();
  await page.evaluate(async () => {
    window.agentTest.state.notReadyReason = null;
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    settingsStore.set(await browser.updateSettings({ connections: [{ id: "test-ai", displayName: "测试连接", kind: "remote", provider: "test", baseUrl: "https://example.invalid", model: "test" }] }));
  });
  await expect(page.getByTestId("agent-not-ready")).toHaveCount(0);
  await page.getByTestId("assistant-request").fill("你好");
  await expect(page.getByTestId("agent-send")).toBeEnabled();
});

test("voice, screen-reading and search costs are listed separately per turn and for the conversation", async ({ page }, testInfo) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    (window as unknown as { costTurn: number }).costTurn = T + 500;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "刚才队友说的什么", source: "wake-voice", atMs: T + 500 } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "他说去下路。", usage: { inputTokens: 2_000, cachedInputTokens: 1_900, outputTokens: 12, cost: 0.002, currency: "CNY" }, atMs: T + 1500 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  // 这一轮：对话（模型）与语音、读屏分开列
  const footer = page.getByTestId("agent-turn").last().getByTestId("agent-turn-usage");
  await expect(footer).toHaveText("输入 2,000（命中缓存 95%） · 输出 12 tokens · 对话 ¥0.0020 · 语音 ¥0.0050 · 读屏 ¥0.0015");
  // 没有这些费用的旧轮次照旧
  await expect(page.getByTestId("agent-turn").first().getByTestId("agent-turn-usage")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("turn-costs.png"), animations: "disabled" });
  // 本段对话：合计含语音、读屏、搜索；明细里分开
  await expect(page.getByTestId("usage-pill-conversation")).toHaveText("本段对话 ¥0.014 · 命中 95%");
  await page.getByTestId("usage-pill").click();
  const dialog = page.getByTestId("usage-dialog");
  await dialog.getByTestId("usage-tab-conversation").click();
  await expect(dialog.getByTestId("usage-conversation-breakdown")).toHaveText("对话 ¥0.0020 · 语音 ¥0.0050 · 读屏 ¥0.0015 · 搜索 ¥0.0055");
  await page.screenshot({ path: testInfo.outputPath("conversation-costs.png"), animations: "disabled" });
});

test("what a background task spent shows up under the turn that handed it off, as 后台", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    const w = window as unknown as { costTurn: number; costBackground: boolean };
    w.costTurn = T + 500;
    w.costBackground = true;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "帮我写个喝药脚本", source: "chat", atMs: T + 500 } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "交给我，好了告诉你。", usage: { inputTokens: 2_000, cachedInputTokens: 1_900, outputTokens: 12, cost: 0.002, currency: "CNY" }, atMs: T + 1500 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  const footer = page.getByTestId("agent-turn").last().getByTestId("agent-turn-usage");
  await expect(footer).toHaveText("输入 2,000（命中缓存 95%） · 输出 12 tokens · 对话 ¥0.0020 · 语音 ¥0.0050 · 后台 ¥0.0030");
  await page.getByTestId("usage-pill").click();
  const dialog = page.getByTestId("usage-dialog");
  await dialog.getByTestId("usage-tab-conversation").click();
  await expect(dialog.getByTestId("usage-conversation-breakdown")).toHaveText("对话 ¥0.0020 · 语音 ¥0.0050 · 后台 ¥0.0030");
});

test("what she said aloud (speak) reads as her words above the details, and an interruption is marked", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.agentTest;
    await t.event("c-old", { kind: "appended", message: { role: "user", text: "寒冰怎么出装", source: "voice", atMs: T + 500 } });
    await t.event("c-old", { kind: "step", step: 1 });
    await t.event("c-old", { kind: "appended", message: {
      role: "assistant", text: "",
      toolCalls: [{ id: "s1", name: "speak", arguments: JSON.stringify({ text: "先出鞋和无尽，详细的我写在聊天里了", written: "1. 狂战士胫甲\n2. 无尽之刃\n3. 电刀" }) }], atMs: T + 1500,
    } });
    await t.event("c-old", { kind: "appended", message: { role: "tool", callId: "s1", name: "speak", content: '{"result":"玩家打断了，只说到：先出鞋和无尽，"}', isError: false, atMs: T + 4000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  const turn = page.getByTestId("agent-turn").last();
  const spoken = turn.getByTestId("agent-spoken");
  await expect(spoken).toContainText("“先出鞋和无尽，详细的我写在聊天里了”");
  await expect(spoken.getByTestId("agent-spoken-interrupted")).toHaveText("被打断了");
  // 不是工具卡片；说的话在正文上面
  await expect(turn.getByTestId("agent-tool-card")).toHaveCount(0);
  // 只写不念的细节（written）显示在下面
  await expect(turn.getByTestId("agent-written")).toContainText("无尽之刃");
  const order = await turn.evaluate((el) => {
    const s = el.querySelector('[data-testid="agent-spoken"]')!;
    const a = el.querySelector('[data-testid="agent-written"]')!;
    return s.compareDocumentPosition(a) & Node.DOCUMENT_POSITION_FOLLOWING;
  });
  expect(order).toBeTruthy();
});

test("where her model used to be (设置 → 她怎么回答 → 她用的模型) now opens the model menu next to the chat box", async ({ page }) => {
  await openChat(page);
  for (const where of ["设置 → 她怎么回答 → 她用的模型", "聊天页 → 她用的模型"]) {
    // 先离开聊天页（去角色页的「交互」），再跳回来
    await openSettingsTab(page, "reply");
    await expect(page.getByTestId("composer-model-menu")).toHaveCount(0);
    await page.evaluate(async (w) => (await import("/src/lib/stores/navigation.ts")).openLocation(w), where);
    await expect(page.getByTestId("assistant-chat"), where).toBeVisible();
    await expect(page.getByTestId("composer-model-menu"), where).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("composer-model-menu"), where).toHaveCount(0);
  }
  // 她的 ui.open 旧地址 ui://kotone/settings/reply：角色页的「交互」
  await page.evaluate(async () => {
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("kotone://ui-open", { uri: "ui://kotone/settings/reply", params: null, title: "reply", plugin: null });
  });
  await expect(page.getByTestId("assistant-nav-reply")).toHaveAttribute("aria-current", "page");
  // 处理完把结果交回后端（工具结果里的 status，ui_open.rs）：玩家在聊天页，她带你过去 = 直接显示
  await expect.poll(() => agentCalls(page, "ui_open_reply")).toEqual([{ uri: "ui://kotone/settings/reply", outcome: "shown" }]);
  // 玩家现在在「交互」：再来一个插件的页面不切过去，交回 waiting
  await page.evaluate(async () => {
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("kotone://ui-open", { uri: "ui://kotone/interpreter/home", params: null, title: "同传", plugin: "interpreter" });
  });
  await expect.poll(() => agentCalls(page, "ui_open_reply")).toHaveLength(2);
  expect((await agentCalls(page, "ui_open_reply"))[1]).toEqual({ uri: "ui://kotone/interpreter/home", outcome: "waiting" });
  await expect(page.getByTestId("assistant-nav-reply")).toHaveAttribute("aria-current", "page");
});
