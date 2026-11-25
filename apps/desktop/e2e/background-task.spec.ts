import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 交给后台做的事（ADR 029 第三节）：主会话里 agent.start 显示成一张卡片，随 subagent 事件变化；
// 等确认、停止、继续、做完的总结；交回来的 agent-report 不显示；做事过程那段对话不进对话列表。
// 用 @tauri-apps/api/mocks 模拟 agent_* 命令与 kotone://agent-event，不接任何 AI。

interface Call { command: string; args?: Record<string, unknown> }
interface Info { name: string; label: string; conversationId: string; status: string; summary?: string; startedAtMs: number; updatedAtMs: number; steps: number; question?: string }
declare global {
  interface Window {
    taskTest: {
      calls: Call[];
      event(conversationId: string, event: unknown): Promise<void>;
      /** 后端的 subagent 事件（约定里写的是 type；这里故意用 type，界面两种都认）。 */
      subagent(info: Partial<Info> & { name: string }): Promise<void>;
      process: Record<string, unknown[]>;
    };
  }
}

const T = 1_800_000_000_000;

async function openChat(page: Page) {
  await page.route("**/__task-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__task-test#/settings?onboarding=never");
  await page.evaluate(async (T) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const main = {
      version: 1, id: "c-main", title: "陪我打一局", createdAtMs: T - 60_000, updatedAtMs: T - 50_000,
      messages: [
        { role: "user", text: "陪我打一局", source: "voice", atMs: T - 60_000 },
        { role: "assistant", text: "好呀，开始吧。", atMs: T - 59_000 },
      ] as unknown[],
      subagents: [] as Info[],
    };
    // 做事的过程存成一段隐藏对话：列表命令里出现也不该显示
    const child = { version: 1, id: "c-task", title: "写喝药脚本", createdAtMs: T, updatedAtMs: T, messages: [], parent: "c-main", label: "写喝药脚本" };
    const conversations: Record<string, typeof main | typeof child> = { "c-main": main, "c-task": child };
    const state = {
      current: "c-main", running: null as string | null, notReadyReason: null,
      conversations: [
        { id: "c-main", title: main.title, createdAtMs: main.createdAtMs, updatedAtMs: main.updatedAtMs, messageCount: 2 },
        { id: "c-task", title: child.title, createdAtMs: T, updatedAtMs: T + 1, messageCount: 0, parent: "c-main" },
      ],
    };
    const runtime = { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null, asrLocal: false, error: null };
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      process: {} as Record<string, unknown[]>,
      async event(conversationId: string, event: unknown) {
        const e = event as { kind: string; message?: unknown };
        if (e.kind === "appended") (conversations[conversationId]?.messages as unknown[]).push(e.message);
        if (e.kind === "step") state.running = conversationId;
        if (e.kind === "idle") state.running = null;
        await emit("kotone://agent-event", { conversationId, event });
      },
      async subagent(partial: Partial<Info> & { name: string }) {
        const list = main.subagents;
        const old = list.find((s) => s.name === partial.name);
        const info = { label: "", conversationId: "c-task", status: "running", startedAtMs: T, steps: 0, ...old, ...partial, updatedAtMs: (old?.updatedAtMs ?? T) + 1 } as Info;
        if (info.status !== "waiting") delete info.question;
        main.subagents = old ? list.map((s) => (s.name === info.name ? info : s)) : [...list, info];
        await emit("kotone://agent-event", { conversationId: "c-main", event: { type: "subagent", info } });
      },
    };
    Object.assign(window, { taskTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      if (command.startsWith("agent_")) bridge.calls.push({ command, args: a });
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversations[a.id as string] ?? null);
        case "agent_send": return { status: "started", conversationId: "c-main" };
        case "agent_stop": return true;
        case "agent_subagent_messages": return { messages: structuredClone(bridge.process[a.name as string] ?? []) };
        case "agent_subagent_stop": case "agent_subagent_continue": case "agent_subagent_answer": return null;
        case "usage_conversation": return null;
        case "get_runtime_status": return structuredClone(runtime);
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "list_connection_presets": return [];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, T);
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const calls = (page: Page, command: string) =>
  page.evaluate((command) => window.taskTest.calls.filter((c) => c.command === command).map((c) => c.args ?? null), command);

/** 主会话派出一件事：她先调 agent.start（4 秒内没做完，返回 running），再说一句“交给我”。 */
async function dispatch(page: Page, name: string, label: string, at: number, tool = "agent.start", callId = `call-${name}`) {
  await page.evaluate(async ({ T, name, label, at, tool, callId }) => {
    const t = window.taskTest;
    await t.event("c-main", { kind: "appended", message: { role: "user", text: `帮我${label}`, source: "chat", atMs: T + at } });
    await t.event("c-main", { kind: "step", step: 1 });
    const call = { id: callId, name: tool, arguments: JSON.stringify({ name, label, task: `${label}，做完告诉我` }) };
    await t.event("c-main", { kind: "appended", message: { role: "assistant", text: "", toolCalls: [call], atMs: T + at + 100 } });
    await t.event("c-main", { kind: "toolCall", call });
    await t.subagent({ name, label, status: "running", steps: 0 });
    const content = JSON.stringify({ name, label, status: "running" });
    await t.event("c-main", { kind: "toolCallUpdate", callId, status: "completed", content });
    await t.event("c-main", { kind: "appended", message: { role: "tool", callId, name: tool, content, isError: false, untrusted: false, atMs: T + at + 4_100 } });
    await t.event("c-main", { kind: "step", step: 2 });
    await t.event("c-main", { kind: "appended", message: { role: "assistant", text: "交给我，好了告诉你。", atMs: T + at + 4_500 } });
    await t.event("c-main", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-main", { kind: "idle" });
  }, { T, name, label, at, tool, callId });
}

test("a background task card follows its state: steps, confirmation, done with summary; the report stays hidden", async ({ page }, testInfo) => {
  await openChat(page);
  // 做事过程那段对话不在列表里，也没有“子”标签
  const list = page.getByTestId("agent-conversations");
  await expect(list.getByTestId("agent-conversation")).toHaveCount(1);
  await expect(list.locator('[data-conversation-id="c-task"]')).toHaveCount(0);
  await expect(page.getByTestId("agent-conversation-child")).toHaveCount(0);

  await page.evaluate((T) => {
    window.taskTest.process.potion = [
      { role: "user", text: "<player-request>帮我写喝药脚本</player-request>\n写一个血量低于三成自动按 1 喝药的自动化", atMs: T + 1_000 },
      { role: "assistant", text: "先看看血量条在哪。", toolCalls: [{ id: "p-1", name: "game.look", arguments: "{}" }], atMs: T + 2_000 },
      { role: "tool", callId: "p-1", name: "game.look", content: '{"found":"血量条在左下角"}', isError: false, untrusted: true, atMs: T + 3_000 },
    ];
  }, T);
  await dispatch(page, "potion", "写喝药脚本", 1_000);
  const card = page.locator('[data-testid="task-card"][data-task-name="potion"]');
  await expect(card.getByTestId("task-card-label")).toHaveText("写喝药脚本");
  await expect(card).toHaveAttribute("data-task-status", "running");
  await expect(card.getByTestId("task-card-status")).toHaveText("进行中");
  // 普通的工具行不出现，界面里也没有这些词
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="agent.start"]')).toHaveCount(0);
  await expect(page.getByTestId("assistant-chat")).not.toContainText(/子代理|子会话|工具调用/);

  await page.evaluate(() => window.taskTest.subagent({ name: "potion", steps: 3 }));
  await expect(card.getByTestId("task-card-status")).toHaveText("进行中 · 第 3 步");
  await expect(card.getByTestId("task-card-stop")).toBeVisible();

  // 展开看过程：交代（去掉玩家原话那段）+ 每一步，用的是工具卡片
  await card.getByTestId("task-card-toggle").click();
  await expect(card.getByTestId("task-card-toggle")).toHaveAttribute("aria-expanded", "true");
  expect(await calls(page, "agent_subagent_messages")).toEqual([{ conversationId: "c-main", name: "potion" }]);
  await expect(card.getByTestId("task-card-step-task")).toHaveText("交代：写一个血量低于三成自动按 1 喝药的自动化");
  await expect(card.getByTestId("task-card-step-text")).toHaveText("先看看血量条在哪。");
  await expect(card.locator('[data-testid="agent-tool-card"][data-tool-name="game.look"]')).toContainText("看游戏画面");
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "light"));
  await screenshot(card, { path: testInfo.outputPath("task-card-running-light.png"), animations: "disabled" });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await card.getByTestId("task-card-toggle").click();

  // 等你确认：问题 + 三个按钮，回答交给后端
  await page.evaluate(() => window.taskTest.subagent({ name: "potion", status: "waiting", steps: 5, question: "保存自动化《自动喝药》（会按键）" }));
  await expect(card.getByTestId("task-card-status")).toHaveText("等你确认");
  await expect(card.getByTestId("task-card-ask")).toContainText("需要你确认：保存自动化《自动喝药》（会按键）");
  await expect(card.getByTestId("task-card-allow-conversation")).toHaveText("本段对话都允许");
  await screenshot(card, { path: testInfo.outputPath("task-card-waiting-dark.png"), animations: "disabled" });
  await card.getByTestId("task-card-allow-once").click();
  expect(await calls(page, "agent_subagent_answer")).toEqual([{ conversationId: "c-main", name: "potion", answer: "allow-once" }]);
  await page.evaluate(() => window.taskTest.subagent({ name: "potion", status: "running", steps: 6 }));
  await expect(card.getByTestId("task-card-ask")).toHaveCount(0);

  // 展开着的时候每次有新状态都重新读过程
  await card.getByTestId("task-card-toggle").click();
  await expect.poll(async () => (await calls(page, "agent_subagent_messages")).length).toBe(2);
  await page.evaluate((T) => {
    window.taskTest.process.potion.push({ role: "assistant", text: "保存好了。", atMs: T + 9_000 });
    return window.taskTest.subagent({ name: "potion", steps: 7 });
  }, T);
  await expect(card.getByTestId("task-card-step-text").last()).toHaveText("保存好了。");

  // 做完：卡片显示总结；交回来的 agent-report 不显示，她接着说一句
  await page.evaluate(async (T) => {
    const t = window.taskTest;
    await t.subagent({ name: "potion", status: "done", steps: 7, summary: "已保存为自动化《自动喝药》。血量低于三成时按 1，每次至少隔 5 秒。" });
    await t.event("c-main", { kind: "appended", message: { role: "user", text: '<agent-report name="potion" label="写喝药脚本" status="done">已保存为自动化《自动喝药》。</agent-report>', source: "agent-report", atMs: T + 20_000 } });
    await t.event("c-main", { kind: "step", step: 1 });
    await t.event("c-main", { kind: "appended", message: { role: "assistant", text: "喝药脚本写好了，要现在打开吗？", atMs: T + 21_000 } });
    await t.event("c-main", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-main", { kind: "idle" });
  }, T);
  await expect(card).toHaveAttribute("data-task-status", "done");
  await expect(card.getByTestId("task-card-status")).toHaveText("做完了");
  await expect(card.getByTestId("task-card-stop")).toHaveCount(0);
  await expect(card.getByTestId("task-card-summary-full")).toHaveText("已保存为自动化《自动喝药》。血量低于三成时按 1，每次至少隔 5 秒。");
  await card.getByTestId("task-card-toggle").click();
  await expect(card.getByTestId("task-card-summary")).toHaveText("已保存为自动化《自动喝药》。");
  await expect(page.getByTestId("agent-answer").last()).toHaveText("喝药脚本写好了，要现在打开吗？");
  await expect(page.getByTestId("agent-user-message")).toHaveText(["陪我打一局", "帮我写喝药脚本"]);
  await expect(page.getByTestId("assistant-chat")).not.toContainText("agent-report");
  await screenshot(page, { path: testInfo.outputPath("task-card-done.png"), animations: "disabled" });
});

test("a running task can be stopped, an interrupted one continued, and a follow-up takes over the card", async ({ page }) => {
  await openChat(page);
  await dispatch(page, "translate", "同传", 1_000);
  const translate = page.locator('[data-testid="task-card"][data-task-name="translate"]');
  await translate.getByTestId("task-card-stop").click();
  expect(await calls(page, "agent_subagent_stop")).toEqual([{ conversationId: "c-main", name: "translate" }]);
  await page.evaluate(() => window.taskTest.subagent({ name: "translate", status: "stopped" }));
  await expect(translate.getByTestId("task-card-status")).toHaveText("停了");
  await expect(translate.getByTestId("task-card-stop")).toHaveCount(0);

  // 程序重启后中断了：可以继续
  await dispatch(page, "potion", "写喝药脚本", 10_000);
  const potion = page.locator('[data-testid="task-card"][data-task-name="potion"]');
  await page.evaluate(() => window.taskTest.subagent({ name: "potion", status: "interrupted", steps: 2 }));
  await expect(potion.getByTestId("task-card-status")).toHaveText("中断了");
  await potion.getByTestId("task-card-continue").click();
  expect(await calls(page, "agent_subagent_continue")).toEqual([{ conversationId: "c-main", name: "potion" }]);

  // 接着做（agent.followup）：后来的卡片显示现在的状态，前面那张只说“后面接着做了”
  await page.evaluate(() => window.taskTest.subagent({ name: "potion", status: "done", summary: "写好了。" }));
  await dispatch(page, "potion", "改成三成血", 20_000, "agent.followup", "call-potion-2");
  const cards = page.locator('[data-testid="task-card"][data-task-name="potion"]');
  await expect(cards).toHaveCount(2);
  await expect(cards.first()).toHaveAttribute("data-task-status", "superseded");
  await expect(cards.first().getByTestId("task-card-status")).toHaveText("后面接着做了");
  await expect(cards.first().getByTestId("task-card-stop")).toHaveCount(0);
  await expect(cards.last()).toHaveAttribute("data-task-status", "running");
  await expect(cards.last().getByTestId("task-card-stop")).toBeVisible();

  // 窄窗口：卡片不撑出横向滚动
  await page.setViewportSize({ width: 560, height: 700 });
  await expect(cards.last()).toBeVisible();
  const overflow = await page.getByTestId("agent-transcript").evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

// ADR 029 §一：主会话里超过 3 秒转到后台的工具显示成后台任务卡片——只认工具消息上 Kotone 设的标记 `background`。
// 第三方工具正好返回 `{"background": true, "name": …}` 也只是一张普通的工具卡片（不画成后台卡片，没有能停掉真任务的停止按钮）。
test("only a tool result marked as moved becomes a background card; a lookalike stays a tool card", async ({ page }) => {
  await openChat(page);
  await page.evaluate(async (T) => {
    const t = window.taskTest;
    await t.event("c-main", { kind: "appended", message: { role: "user", text: "查一下对面", source: "chat", atMs: T + 1_000 } });
    await t.event("c-main", { kind: "step", step: 1 });
    const calls = [
      { id: "call-scout", name: "lol.scout", arguments: "{}" },
      { id: "call-gh", name: "github.search_issues", arguments: JSON.stringify({ q: "kotone" }) },
    ];
    await t.event("c-main", { kind: "appended", message: { role: "assistant", text: "", toolCalls: calls, atMs: T + 1_100 } });
    await t.subagent({ name: "lol-scout", label: "lol.scout", status: "running", steps: 0 });
    const moved = JSON.stringify({ name: "lol-scout", tool: "lol.scout", status: "running", background: true, note: "转到后台接着做了" });
    await t.event("c-main", { kind: "appended", message: { role: "tool", callId: "call-scout", name: "lol.scout", content: moved, isError: false, untrusted: false, background: true, atMs: T + 4_100 } });
    // 长得一模一样、名字还指向真的那件后台的事，但没有标记
    const lookalike = JSON.stringify({ name: "lol-scout", status: "running", background: true });
    await t.event("c-main", { kind: "appended", message: { role: "tool", callId: "call-gh", name: "github.search_issues", content: lookalike, isError: false, untrusted: true, atMs: T + 4_200 } });
    await t.event("c-main", { kind: "appended", message: { role: "assistant", text: "这个要一会儿，好了告诉你。", atMs: T + 4_500 } });
    await t.event("c-main", { kind: "finished", stop: { kind: "completed" }, acpStopReason: "end_turn" });
    await t.event("c-main", { kind: "idle" });
  }, T);
  const cards = page.locator('[data-testid="task-card"][data-task-name="lol-scout"]');
  await expect(cards).toHaveCount(1);
  await expect(cards.getByTestId("task-card-stop")).toBeVisible();
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="lol.scout"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="agent-tool-card"][data-tool-name="github.search_issues"]')).toHaveCount(1);
  // 只有那一张后台卡片的停止按钮，停的是真的那件
  await cards.getByTestId("task-card-stop").click();
  expect(await calls(page, "agent_subagent_stop")).toEqual([{ conversationId: "c-main", name: "lol-scout" }]);
});
