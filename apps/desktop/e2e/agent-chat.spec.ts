import { expect, test, type Page } from "@playwright/test";

// 「琴音 → 聊天」：用 @tauri-apps/api/mocks 模拟 agent_* 命令与 kotone://agent-* 事件，不接任何 AI。

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

async function openChat(page: Page, options: { empty?: boolean; notReady?: string } = {}) {
  await page.route("**/__agent-chat-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__agent-chat-test#/settings?onboarding=never");
  await page.evaluate(async ({ empty, notReady, T }) => {
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
    let connectionModel = "test";
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[], state, conversations,
      async event(conversationId: string, event: unknown) {
        // 与后端一致：每条 appended 的消息同时存盘
        const e = event as { kind: string; message?: unknown };
        if (e.kind === "appended") conversations[conversationId]?.messages.push(e.message as never);
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
      if (command.startsWith("agent_")) bridge.calls.push({ command, args: args as Record<string, unknown> });
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
        case "get_runtime_status": return { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null };
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
  }, { empty: options.empty ?? false, notReady: options.notReady, T });
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const agentCalls = (page: Page, command: string) =>
  page.evaluate((command) => window.agentTest.calls.filter((call) => call.command === command).map((call) => call.args ?? null), command);

test("empty conversation shows starter prompts; a starter sends or fills the composer", async ({ page }, testInfo) => {
  await openChat(page, { empty: true });
  const starters = page.getByTestId("agent-starter");
  await expect(starters).toHaveText(["帮我配置 Kotone", "做一个自动化", "看看队友在说什么", "设个提醒", "我有哪些自动化"]);
  await page.screenshot({ path: testInfo.outputPath("agent-chat-empty.png"), animations: "disabled" });
  // 以冒号结尾的提示只放进输入框
  await starters.filter({ hasText: "做一个自动化" }).click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("帮我做一个自动化：");
  expect(await agentCalls(page, "agent_send")).toEqual([]);
  // 其它提示直接发出，并切到新建的对话
  await starters.filter({ hasText: "看看队友在说什么" }).click();
  expect(await agentCalls(page, "agent_send")).toEqual([{ text: "看看队友在聊天里说了什么" }]);
  await expect(page.getByTestId("agent-conversation-title")).toHaveText(/看看队友在聊天里说了什么/);
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
    await t.event("c-old", { kind: "appended", message: { role: "assistant", text: "已经发了。", usage: { inputTokens: 1400, outputTokens: 8, cachedInputTokens: 1000 }, atMs: T + 3000 } });
    await t.event("c-old", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-old", { kind: "idle" });
  }, T);
  await expect(page.getByTestId("agent-stop")).toHaveCount(0);
  await expect(page.getByTestId("agent-send")).toHaveText("发送");
  const footer = page.getByTestId("agent-turn").last().getByTestId("agent-turn-usage");
  await expect(footer).toHaveText("输入 2,600（缓存 1,000） · 输出 38 tokens");
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
  await expect(page.getByTestId("agent-user-source").last()).toHaveText("游戏内 · 语音");
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

test("not ready shows the reason and a way to settings", async ({ page }) => {
  await openChat(page, { notReady: "请先在「琴音 → 设置」里选择 AI 连接" });
  await expect(page.getByTestId("agent-not-ready")).toContainText("请先在「琴音 → 设置」里选择 AI 连接");
  await page.getByTestId("assistant-request").fill("你好");
  await expect(page.getByTestId("agent-send")).toBeDisabled();
  await page.getByTestId("agent-open-settings").click();
  await expect(page.getByTestId("assistant-nav-settings")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("agent-settings")).toBeVisible();
});

test("agent settings replace permissions as a whole and save the in-game switch", async ({ page }) => {
  await openChat(page);
  await page.getByTestId("assistant-nav-settings").click();
  const settings = page.getByTestId("agent-settings");
  await settings.getByTestId("agent-mode-read-only").click();
  await expect(settings.getByTestId("agent-mode-read-only")).toHaveAttribute("aria-pressed", "true");
  await settings.getByTestId("agent-rules").locator("summary").click();
  // 工具清单来自 agent_tools：按分组列出，显示默认规则
  await expect(settings.getByTestId("agent-rule")).toHaveCount(3);
  await expect(settings.locator('[data-tool="game_chat.send"]').getByTestId("agent-rule-default")).toHaveText("默认：询问");
  await expect(settings.getByRole("heading", { name: "屏幕与窗口", exact: true })).toBeVisible();
  await settings.getByLabel("发到游戏聊天 的规则").click();
  await page.getByRole("option", { name: "禁止" }).click();
  await expect(settings.getByLabel("发到游戏聊天 的规则")).toHaveText("禁止");
  await settings.getByLabel("发到游戏聊天 的规则").click();
  await page.getByRole("option", { name: "按默认" }).click();
  // 每次都整体替换：删掉的规则不会留在补丁里
  expect(await agentCalls(page, "agent_set_permissions")).toEqual([
    { permissions: { mode: "read-only", rules: {} } },
    { permissions: { mode: "read-only", rules: { "game_chat.send": "deny" } } },
    { permissions: { mode: "read-only", rules: {} } },
  ]);
  await settings.getByTestId("agent-in-game").click();
  await expect(settings.getByTestId("agent-in-game")).toHaveAttribute("aria-checked", "false");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
  expect(saved.agent?.inGame).toBe(false);
});

test("composer switches permission mode and model like DeepSeek Harness", async ({ page }, testInfo) => {
  await openChat(page);
  const permission = page.getByTestId("composer-permission");
  await expect(permission).toContainText("先问我");
  await permission.click();
  await page.getByTestId("composer-permission-allow-all").click();
  await expect(permission).toContainText("完全权限");
  expect(await agentCalls(page, "agent_set_permissions")).toEqual([{ permissions: { mode: "allow-all", rules: {} } }]);

  const model = page.getByTestId("composer-model");
  await expect(model).toContainText("test");
  await model.click();
  // 模型清单来自服务商的模型列表（照 DeepSeek Harness），不是写死的预置名
  await expect(page.getByTestId("composer-model-test-ai-test-listed")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("composer-model-menu.png"), animations: "disabled" });
  await page.getByTestId("composer-model-test-ai-test-pro").click();
  await expect(model).toContainText("test-pro");
  const calls = await page.evaluate(() => window.agentTest.calls.filter((c) => c.command === "upsert_connection" || c.command === "connection_self_check"));
  // 换模型不再自检
  expect(calls.map((c) => c.command)).toEqual(["upsert_connection"]);
  expect((calls[0].args as { connection: { model: string } }).connection.model).toBe("test-pro");
});
