// 内置琴音 Agent（ADR 014 / 015）：对话、事件与权限询问。
// 事件按 ACP 的会话更新设计（消息片段、思考片段、工具调用及状态、权限请求）。
// 后端：apps/desktop/src-tauri/src/agent.rs。浏览器预览只展示示例对话，不调用任何 AI。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import { m } from "./paraglide/messages.js";
import { sampleMatch } from "./ipc/lolMock";
import { findSampleChampion, sampleChampionRift, sampleTierRift } from "./ipc/lolStatsMock";

export interface ToolCall { id: string; name: string; arguments: string }
/** 插件工具结果里给界面的那份（照 MCP Apps：structuredContent + 页面地址）。 */
export interface ToolDisplay { structuredContent?: unknown; ui: { uri: string; params?: Record<string, unknown> | null } }
/** `cachedInputTokens`：输入里命中服务商前缀缓存的部分；`cost` + `currency`：按 Kotone 价格表估的费用（DeepSeek 用人民币）。 */
export interface TokenUsage { inputTokens?: number; outputTokens?: number; cachedInputTokens?: number; reasoningTokens?: number; costUsd?: number; cost?: number; currency?: string }
export type Attachment =
  | { kind: "data"; label: string; text: string }
  | { kind: "image"; label: string; mime: string; data?: string };

export type AgentMessage =
  | { role: "user"; text: string; attachments?: Attachment[]; source?: string; atMs: number; context?: string }
  | { role: "assistant"; text: string; toolCalls?: ToolCall[]; reasoning?: string; usage?: TokenUsage; atMs: number }
  /** `display`：插件工具带页面的结果（ADR 038，照 MCP Apps）——给页面的数据和用哪个页面，只给界面，她看不到。 */
  | { role: "tool"; callId: string; name: string; content: string; isError: boolean; untrusted: boolean; atMs: number; display?: ToolDisplay }
  /** `compaction`：上下文压缩的检查点（摘要）；它之前的消息不再发给模型。 */
  /** `recap`：对局复盘的正文（`text` 是标题），只给玩家看。 */
  | { role: "notice"; text: string; atMs: number; compaction?: string; usage?: TokenUsage; recap?: string };

/**
 * 交给后台做的事（ADR 029 第三节）：主会话用 `agent.start` 派出去，玩家只看到主会话里的一张卡片。
 * `waiting` = 在等玩家确认（`question` 是问的那句话）。
 */
export type SubagentStatus = "running" | "waiting" | "done" | "failed" | "stopped" | "interrupted";
export interface SubagentInfo {
  name: string;
  label: string;
  /** 它做事过程的那段隐藏对话。 */
  conversationId: string;
  status: SubagentStatus;
  /** done / failed 时的总结。 */
  summary?: string;
  startedAtMs: number;
  updatedAtMs: number;
  /** 已经做了几步。 */
  steps: number;
  question?: string;
}
/** 卡片上的确认按钮（`agent_subagent_answer`）。 */
export type SubagentAnswer = "allow-once" | "allow-conversation" | "deny";

/** `parent` / `label`：后台做事的过程（ADR 029），不出现在对话列表里。`subagents`：主会话派出去的事。 */
export interface Conversation { version: number; id: string; title: string; createdAtMs: number; updatedAtMs: number; messages: AgentMessage[]; parent?: string | null; label?: string | null; subagents?: SubagentInfo[] }
export interface ConversationSummary { id: string; title: string; createdAtMs: number; updatedAtMs: number; messageCount: number; parent?: string | null }

export type StopReason = { kind: "completed" | "truncated" | "step-limit" | "time-limit" | "cancelled" } | { kind: "failed"; message: string };
export type ToolStatus = "pending" | "in_progress" | "completed" | "failed";

export type AgentEvent =
  | { kind: "step"; step: number }
  | { kind: "chunk"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "toolCall"; call: ToolCall }
  | { kind: "toolCallUpdate"; callId: string; status: ToolStatus; content: string | null }
  | { kind: "appended"; message: AgentMessage }
  | { kind: "compacted"; index: number; message: AgentMessage }
  /** 空回复已丢弃、重发了同样的请求。 */
  | { kind: "discarded" }
  | { kind: "finished"; stop: StopReason; acpStopReason: string | null }
  /** 派出去的一件事有了新状态（ADR 029）：按 name 更新 `conversation.subagents`。 */
  | { kind: "subagent"; info: SubagentInfo }
  | { kind: "idle" };

export interface PermissionRequest { callId: string; tool: string; title: string; reason: string; arguments: unknown }
export interface AgentAsk { askId: string; conversationId: string; request: PermissionRequest }
export type Answer = "allow-once" | "allow-for-conversation" | "deny";

export type PermissionMode = "default" | "allow-all" | "read-only";
export type PermissionRule = "allow" | "ask" | "deny";
export interface Permissions { mode: PermissionMode; rules: Record<string, PermissionRule> }
export interface AgentState {
  current: string | null;
  running: string | null;
  conversations: ConversationSummary[];
  notReadyReason: string | null;
  permissions: Permissions;
}

/** agent_tools 的一项：琴音能用的工具、默认规则与用户设的规则（null = 按默认）。 */
export interface AgentTool {
  name: string;
  group: string;
  description: string;
  sideEffects: string[];
  defaultRule: "allow" | "ask";
  rule: PermissionRule | null;
  /** 始终提供给模型的核心工具（其余经 tools.find 启用）。 */
  core: boolean;
}

export const PREVIEW_ERROR = m.chat_preview_error();

// ---------- 浏览器预览数据（只在 isTauri = false 时使用） ----------

const t0 = Date.now() - 5 * 60_000;
const previewConversation: Conversation = {
  // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的数据），不是界面文字
  version: 1, id: "c-preview", title: "帮我做一个每秒按 Q 的自动化", createdAtMs: t0, updatedAtMs: t0 + 60_000,
  messages: [
    { role: "user", text: "帮我做一个每秒按 Q 的自动化", source: "chat", atMs: t0 },
    { role: "assistant", text: "好的，我先试运行一下，确认没问题再帮你保存。", atMs: t0 + 2_000,
      reasoning: "用户想要每秒按一次 Q。先用 automation.run 试运行，确认目标窗口正确再保存。",
      toolCalls: [{ id: "call-1", name: "automation.run", arguments: '{"name":"连按Q","source":"every(1000, function() key(\\"Q\\") end)"}' }],
      usage: { inputTokens: 1830, outputTokens: 96 } },
    { role: "tool", callId: "call-1", name: "automation.run", content: '{"status":"started","id":"run-7","target":"英雄联盟 (League of Legends.exe)"}', isError: false, untrusted: false, atMs: t0 + 9_000 },
    { role: "assistant", text: "已经开始了：**每秒按一次 Q**，按 `Ctrl+Shift+F12` 可以随时停止。\n\n要我把它保存下来吗？", atMs: t0 + 11_000,
      usage: { inputTokens: 1990, outputTokens: 41 } },
    { role: "user", text: "（系统消息，不是玩家说的话：自动化「连按Q」已停止。执行记录见附件；需要的话告诉玩家结果。）", source: "automation", atMs: t0 + 53_000,
      attachments: [{ kind: "data", label: "执行记录", text: '{"id":"run-7","status":"cancelled","durationMs":42000}' }] },
    { role: "assistant", text: "连按 Q 已经停了，一共跑了 42 秒。需要的话随时叫我再开。", atMs: t0 + 55_000 },
    { role: "user", text: "再帮我写个血量低于三成就自动喝药的脚本", source: "voice", atMs: t0 + 120_000 },
    { role: "assistant", text: "", atMs: t0 + 121_000,
      toolCalls: [{ id: "call-2", name: "agent.start", arguments: '{"name":"potion","label":"写喝药脚本","task":"写一个血量低于三成自动按 1 喝药的自动化"}' }] },
    { role: "tool", callId: "call-2", name: "agent.start", content: '{"name":"potion","label":"写喝药脚本","status":"running"}', isError: false, untrusted: false, atMs: t0 + 125_000 },
    { role: "assistant", text: "交给我，写好了告诉你。", atMs: t0 + 125_500 },
    { role: "user", text: '<agent-report name="potion" label="写喝药脚本" status="done">已保存为自动化《自动喝药》。</agent-report>', source: "agent-report", atMs: t0 + 190_000 },
    { role: "assistant", text: "喝药脚本写好了，保存成了《自动喝药》，要现在打开吗？", atMs: t0 + 191_000 },
    // 插件工具带页面的结果（ADR 038）：聊天里出一张卡片，点开在右侧栏
    { role: "user", text: "查一下这把的战绩，对面谁是突破口", source: "chat", atMs: t0 + 240_000 },
    { role: "assistant", text: "", atMs: t0 + 241_000, toolCalls: [{ id: "call-3", name: "lol.scout", arguments: "{}" }] },
    { role: "tool", callId: "call-3", name: "lol.scout", content: '{"status":"ok","text":"每人最近 20 局……突破口（对面，按数据）：亚索——6 连败"}', isError: false, untrusted: true, atMs: t0 + 242_000,
      display: { structuredContent: sampleMatch(), ui: { uri: "ui://kotone/lol/match" } } },
    { role: "assistant", text: "对面亚索最近 20 局只赢 6 局，刚 6 连败，可以多找他。", atMs: t0 + 244_000 },
    // 网上的统计（ADR 038 第 3 步）：英雄卡片
    { role: "user", text: "亚索克制谁", source: "chat", atMs: t0 + 260_000 },
    { role: "assistant", text: "", atMs: t0 + 261_000, toolCalls: [{ id: "call-4", name: "lol.champion", arguments: '{"champion":"亚索"}' }] },
    { role: "tool", callId: "call-4", name: "lol.champion", content: '{"status":"ok","text":"亚索 上路（16.23 版本，国服全段位）：T1……"}', isError: false, untrusted: true, atMs: t0 + 262_000,
      display: { structuredContent: sampleChampionRift(findSampleChampion("亚索")!), ui: { uri: "ui://kotone/lol/champion" } } },
    { role: "assistant", text: "掌盟的数据：亚索上路怕贾克斯、雷克顿、孙悟空，对斯莫德、斯维因好打。", atMs: t0 + 264_000 },
  ],
  subagents: [{
    name: "potion", label: "写喝药脚本", conversationId: "c-preview-potion", status: "done",
    summary: "已保存为自动化《自动喝药》。血量条低于三成时按 1，每次之间至少隔 5 秒；按 Ctrl+Shift+F12 随时停。",
    startedAtMs: t0 + 121_000, updatedAtMs: t0 + 190_000, steps: 6,
  }],
};
/** 浏览器预览里“写喝药脚本”的过程。 */
const previewTaskMessages: AgentMessage[] = [
  { role: "user", text: "<player-request>再帮我写个血量低于三成就自动喝药的脚本</player-request>\n写一个血量低于三成自动按 1 喝药的自动化", atMs: t0 + 121_000 },
  { role: "assistant", text: "先看看血量条在哪。", atMs: t0 + 126_000, toolCalls: [{ id: "p-1", name: "game.look", arguments: "{}" }] },
  { role: "tool", callId: "p-1", name: "game.look", content: '{"window":"英雄联盟","found":"血量条在左下角"}', isError: false, untrusted: true, atMs: t0 + 130_000 },
  { role: "assistant", text: "", atMs: t0 + 150_000, toolCalls: [{ id: "p-2", name: "automation.save", arguments: '{"name":"自动喝药"}' }] },
  { role: "tool", callId: "p-2", name: "automation.save", content: '{"saved":true,"id":"auto-potion"}', isError: false, untrusted: false, atMs: t0 + 185_000 },
  { role: "assistant", text: "已保存为自动化《自动喝药》。血量条低于三成时按 1，每次之间至少隔 5 秒；按 Ctrl+Shift+F12 随时停。", atMs: t0 + 190_000 },
];
const preview = {
  current: previewConversation.id as string | null,
  conversations: [previewConversation],
  permissions: { mode: "default", rules: {} } as Permissions,
  hidden: [] as string[],
};
/** 浏览器预览的工具清单（与后端分组、默认规则一致的示例）。 */
const previewTools: Omit<AgentTool, "rule">[] = [
  { name: "speak", group: "game", description: "对玩家说一句话（只有这里的字会念出来）", sideEffects: [], defaultRule: "allow", core: true },
  { name: "reminder.create", group: "game", description: "过一会儿提醒玩家", sideEffects: [], defaultRule: "allow", core: true },
  { name: "game_chat.send", group: "game", description: "把一句话打进游戏聊天框并发出", sideEffects: ["input"], defaultRule: "ask", core: true },
  { name: "companion.read_chat", group: "game", description: "截取游戏窗口，识别聊天记录", sideEffects: [], defaultRule: "allow", core: true },
    // 强度榜是插件的 MCP App（ADR 039 D5）：卡片里是同一个 App（inline）
    { role: "user", text: "现在上单什么强", source: "chat", atMs: t0 + 280_000 },
    { role: "assistant", text: "", atMs: t0 + 281_000, toolCalls: [{ id: "call-5", name: "lol.tier", arguments: '{"lane":"上"}' }] },
    { role: "tool", callId: "call-5", name: "lol.tier", content: '{"status":"ok","text":"上路：……"}', isError: false, untrusted: true, atMs: t0 + 282_000,
      display: { structuredContent: sampleTierRift("TOP"), ui: { uri: "ui://kotone/lol/tier", params: { lane: "上" } } } },
    { role: "assistant", text: "掌盟的数据：上路的强度榜在这里。", atMs: t0 + 284_000 },
  { name: "game.list", group: "game", description: "列出已配置的游戏和它们是否在运行", sideEffects: [], defaultRule: "allow", core: true },
  { name: "game.focus", group: "game", description: "切换到游戏窗口", sideEffects: ["window"], defaultRule: "allow", core: true },
  { name: "game.launch", group: "game", description: "打开一个已配置启动路径的游戏", sideEffects: ["process"], defaultRule: "ask", core: false },
  { name: "game.look", group: "game", description: "看游戏画面（截图 + 文字编号）", sideEffects: ["captures-screen"], defaultRule: "allow", core: true },
  { name: "game.act", group: "game", description: "在游戏里操作鼠标键盘（一步一步）", sideEffects: ["sends-game-input"], defaultRule: "ask", core: true },
  { name: "window.active", group: "screen", description: "看当前前台窗口", sideEffects: [], defaultRule: "allow", core: true },
  { name: "ocr.recognize", group: "screen", description: "识别屏幕上一块区域的文字", sideEffects: [], defaultRule: "allow", core: false },
  { name: "automation.list", group: "automation", description: "列出已保存的自动化", sideEffects: [], defaultRule: "allow", core: true },
  { name: "automation.run", group: "automation", description: "运行一段自动化（会按键、点鼠标）", sideEffects: ["input"], defaultRule: "ask", core: true },
  { name: "automation.stop", group: "automation", description: "停止正在运行的自动化", sideEffects: [], defaultRule: "allow", core: true },
  { name: "automation.save", group: "automation", description: "保存一段自动化", sideEffects: ["write"], defaultRule: "ask", core: true },
  { name: "settings.update", group: "settings", description: "修改 Kotone 设置", sideEffects: ["write"], defaultRule: "ask", core: false },
  // i18n-ignore-end
];

function summary(c: Conversation): ConversationSummary {
  return { id: c.id, title: c.title, createdAtMs: c.createdAtMs, updatedAtMs: c.updatedAtMs, messageCount: c.messages.length };
}

// ---------- 命令 ----------

export async function agentState(): Promise<AgentState> {
  if (!isTauri) {
    return {
      current: preview.current, running: null, notReadyReason: null,
      conversations: preview.conversations.map(summary),
      permissions: structuredClone(preview.permissions),
    };
  }
  return invoke("agent_state");
}
export async function agentConversation(id: string): Promise<Conversation | null> {
  if (!isTauri) {
    const found = preview.conversations.find((c) => c.id === id);
    return found ? structuredClone(found) : null;
  }
  return invoke("agent_conversation", { id });
}
/** 发一条消息；琴音正在回答时算“插话”（status = steered）。 */
export async function agentSend(text: string): Promise<{ status: "started" | "steered"; conversationId: string }> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  return invoke("agent_send", { text });
}
/** 停止当前这一轮（同时清掉没处理的插话 / 排队）。 */
export async function agentStop(): Promise<boolean> {
  return isTauri ? invoke("agent_stop") : false;
}
export async function agentAnswer(askId: string, answer: Answer): Promise<boolean> {
  return isTauri ? invoke("agent_answer", { askId, answer }) : false;
}
/** 下一条消息开始新对话（正在回答时报错）。 */
export async function agentNewConversation(): Promise<void> {
  if (isTauri) await invoke("agent_new_conversation");
  else preview.current = null;
}
export async function agentSelectConversation(id: string): Promise<void> {
  if (isTauri) await invoke("agent_select_conversation", { id });
  else preview.current = id;
}
export async function agentDeleteConversation(id: string): Promise<boolean> {
  if (isTauri) return invoke("agent_delete_conversation", { id });
  const before = preview.conversations.length;
  preview.conversations = preview.conversations.filter((c) => c.id !== id);
  if (preview.current === id) preview.current = null;
  return preview.conversations.length !== before;
}
/** 整体替换权限设置（设置补丁是递归合并的，删不掉单条规则，所以不用 update_settings）。 */
export async function agentSetPermissions(permissions: Permissions): Promise<Permissions> {
  if (isTauri) return invoke("agent_set_permissions", { permissions });
  preview.permissions = structuredClone(permissions);
  return structuredClone(preview.permissions);
}
/** 某个已保存的自动化交不交给琴音；返回新的隐藏列表。 */
export async function agentSetAutomationHidden(id: string, hidden: boolean): Promise<string[]> {
  if (isTauri) return invoke("agent_set_automation_hidden", { id, hidden });
  preview.hidden = preview.hidden.filter((x) => x !== id);
  if (hidden) preview.hidden.push(id);
  return [...preview.hidden];
}

/** 琴音能用的全部工具（设置页按工具设规则用）。 */
export async function agentTools(): Promise<AgentTool[]> {
  if (isTauri) return (await invoke<{ tools: AgentTool[] }>("agent_tools")).tools;
  return previewTools.map((tool) => ({ ...tool, rule: preview.permissions.rules[tool.name] ?? null }));
}

// ---------- 交给后台做的事（ADR 029 第三节） ----------

/** 一件事的过程（和聊天时间线同一种消息），卡片展开时读。 */
export async function agentSubagentMessages(conversationId: string, name: string): Promise<AgentMessage[]> {
  if (!isTauri) {
    const known = preview.conversations.find((c) => c.id === conversationId)?.subagents?.some((s) => s.name === name);
    return known ? structuredClone(previewTaskMessages) : [];
  }
  return (await invoke<{ messages: AgentMessage[] }>("agent_subagent_messages", { conversationId, name })).messages;
}
export async function agentSubagentStop(conversationId: string, name: string): Promise<void> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  await invoke("agent_subagent_stop", { conversationId, name });
}
/** 程序重启后中断了的事：接着做。 */
export async function agentSubagentContinue(conversationId: string, name: string): Promise<void> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  await invoke("agent_subagent_continue", { conversationId, name });
}
export async function agentSubagentAnswer(conversationId: string, name: string, answer: SubagentAnswer): Promise<void> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  await invoke("agent_subagent_answer", { conversationId, name, answer });
}

/** 后端事件统一成 `kind`（约定里新事件写的是 `type`，两种都认）。 */
function normalizeEvent(event: AgentEvent | (Record<string, unknown> & { type?: string })): AgentEvent {
  const raw = event as Record<string, unknown>;
  if (raw.kind === undefined && typeof raw.type === "string") return { ...raw, kind: raw.type } as unknown as AgentEvent;
  return event as AgentEvent;
}

export async function subscribeAgent(callback: (conversationId: string, event: AgentEvent) => void): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  return listen<{ conversationId: string; event: AgentEvent }>("kotone://agent-event", (e) => callback(e.payload.conversationId, normalizeEvent(e.payload.event)));
}
export async function subscribeAsks(onAsk: (ask: AgentAsk) => void, onResolved: (askId: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  const a = await listen<AgentAsk>("kotone://agent-ask", (e) => onAsk(e.payload));
  const b = await listen<{ askId: string }>("kotone://agent-ask-resolved", (e) => onResolved(e.payload.askId));
  return () => { a(); b(); };
}

/** 琴音请玩家粘贴某个服务商的 Key（connections.request_key）：Key 直接存进凭据库，不经过模型。 */
export interface KeyRequest {
  requestId: string; conversationId: string; provider: string; displayName: string;
  apiKeyUrl: string | null; connectionId: string; reason: string;
}
export async function subscribeKeyRequests(onRequest: (request: KeyRequest) => void, onResolved: (requestId: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  const a = await listen<KeyRequest>("kotone://agent-key-request", (e) => onRequest(e.payload));
  const b = await listen<{ requestId: string }>("kotone://agent-key-request-resolved", (e) => onResolved(e.payload.requestId));
  return () => { a(); b(); };
}
/** 提交（apiKey 为空 = 不用了）。 */
export async function agentSubmitKey(request: KeyRequest, apiKey: string | null): Promise<{ saved: boolean; connectionId?: string }> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  return invoke("agent_submit_key", { requestId: request.requestId, provider: request.provider, connectionId: request.connectionId || null, apiKey });
}

// ---------- 展示 ----------

/** 工具名的中文说明（卡片标题用；没有的显示原名）。 */
const toolLabels: Record<string, string> = {
  "tools.find": m.chat_tool_label_tools_find(),
  speak: m.chat_tool_label_speak(),
  danmaku: m.chat_tool_label_danmaku(),
  "reply.say": m.chat_tool_label_reply_say(),
  "reminder.create": m.chat_tool_label_reminder_create(),
  "web.search": m.chat_tool_label_web_search(),
  "web.fetch": m.chat_tool_label_web_fetch(),
  "game_chat.send": m.chat_tool_label_game_chat_send(),
  "companion.read_chat": m.chat_tool_label_companion_read_chat(),
  "window.active": m.chat_tool_label_window_active(),
  "window.list": m.chat_tool_label_window_list(),
  "game.list": m.chat_tool_label_game_list(),
  "game.focus": m.chat_tool_label_game_focus(),
  "game.launch": m.chat_tool_label_game_launch(),
  "game.close": m.chat_tool_label_game_close(),
  "game.look": m.chat_tool_label_game_look(),
  "game.act": m.chat_tool_label_game_act(),
  "automation.list": m.chat_tool_label_automation_list(),
  "automation.get": m.chat_tool_label_automation_get(),
  "automation.run": m.chat_tool_label_automation_run(),
  "automation.runs": m.chat_tool_label_automation_runs(),
  "automation.stop": m.chat_tool_label_automation_stop(),
  "automation.save": m.chat_tool_label_automation_save(),
  "automation.delete": m.chat_tool_label_automation_delete(),
  "settings.get": m.chat_tool_label_settings_get(),
  "settings.update": m.chat_tool_label_settings_update(),
  "ocr.recognize": m.chat_tool_label_ocr_recognize(),
  "settings.describe": m.chat_tool_label_settings_describe(),
  "game.installed": m.chat_tool_label_game_installed(),
  "game.state": m.chat_tool_label_game_state(),
  "models.list": m.chat_tool_label_models_list(),
  "models.download": m.chat_tool_label_models_download(),
  "models.activate": m.chat_tool_label_models_activate(),
  "models.delete": m.chat_tool_label_models_delete(),
  "speech.model.download": m.chat_tool_label_speech_model_download(),
  "connections.list": m.chat_tool_label_connections_list(),
  "connections.presets": m.chat_tool_label_connections_presets(),
  "connections.request_key": m.chat_tool_label_connections_request_key(),
  "wake.status": m.chat_tool_label_wake_status(),
  "wake.model.download": m.chat_tool_label_wake_model_download(),
  "runtime.status": m.chat_tool_label_runtime_status(),
  "runtime.start": m.chat_tool_label_runtime_start(),
  "app.restart": m.chat_tool_label_app_restart(),
  "memory": m.chat_tool_label_memory(),
  "skill.view": m.chat_tool_label_skill_view(),
  "skill.manage": m.chat_tool_label_skill_manage(),
  "soul.write": m.chat_tool_label_soul_write(),
  "session.spawn": m.chat_tool_label_agent_start(),
  "session.send": m.chat_tool_label_session_send(),
  "session.list": m.chat_tool_label_agent_list(),
  "session.history": m.chat_tool_label_session_history(),
  "session.stop": m.chat_tool_label_agent_stop(),
  "agent.start": m.chat_tool_label_agent_start(),
  "agent.followup": m.chat_tool_label_agent_followup(),
  "agent.list": m.chat_tool_label_agent_list(),
  "agent.stop": m.chat_tool_label_agent_stop(),
  "voice.listen.start": m.chat_tool_label_voice_listen_start(),
  "voice.listen.stop": m.chat_tool_label_voice_listen_stop(),
  "ask": m.chat_tool_label_ask(),
  // ADR 030：后台在电脑上做事
  "shell.run": m.chat_tool_label_shell_run(),
  "file.read": m.chat_tool_label_file_read(),
  "file.write": m.chat_tool_label_file_write(),
  "file.edit": m.chat_tool_label_file_edit(),
  "file.find": m.chat_tool_label_file_find(),
};
export function toolLabel(name: string): string {
  if (name.startsWith("saved.")) return m.chat_tool_label_saved();
  return toolLabels[name] ?? name;
}
/** 工具分组的中文名（agent_tools 的 group；没有的显示原名）。 */
export const toolGroupLabels: Record<string, string> = {
  game: m.chat_tool_group_game(),
  automation: m.chat_tool_group_automation(),
  screen: m.chat_tool_group_screen(),
  settings: m.chat_tool_group_settings(),
  models: m.chat_tool_group_models(),
  meta: m.chat_tool_group_meta(),
  computer: m.chat_tool_group_computer(),
  app: m.chat_tool_group_app(),
};
export function toolGroupLabel(group: string): string {
  return toolGroupLabels[group] ?? group;
}

export function stopText(stop: StopReason): string | null {
  switch (stop.kind) {
    case "completed": return null;
    case "truncated": return m.chat_stop_reason_truncated();
    case "step-limit": return m.chat_stop_reason_step_limit();
    case "time-limit": return m.chat_stop_reason_time_limit();
    case "cancelled": return m.chat_stop_reason_cancelled();
    case "failed": return m.chat_stop_reason_failed({ message: stop.message });
  }
}

/** 用户消息的来源说明；聊天窗口里打的字不显示。 */
export function sourceLabel(source: string | undefined): string | null {
  switch (source) {
    case undefined: case "": case "chat": return null;
    case "assistant-text": return m.chat_source_assistant_text();
    // 唤醒词、回答完接着听（ADR 029）都是语音叫她，和呼叫键一样显示
    case "voice": case "wake-voice": return m.chat_source_voice();
    case "quick-command": return m.chat_source_quick_command();
    case "tool": return m.chat_source_tool();
    case "automation": return m.chat_source_automation();
    // 后台做完的事交回来（ADR 029）：卡片已经更新，这条不显示
    case "agent-report": return null;
    default: return source;
  }
}

/** 起手提示（ADR 014“已确认”）。 */
/** 聊天页的起手提示（ADR 028）。以冒号结尾的放进输入框让玩家接着写，其余直接发出。 */
export const starterPrompts = [
  { label: m.chat_starter_what_can_you_do(), text: m.chat_starter_what_can_you_do() },
  { label: m.chat_starter_setup_label(), text: m.chat_starter_setup_text() },
  { label: m.chat_starter_strong_heroes(), text: m.chat_starter_strong_heroes() },
  { label: m.chat_starter_song_label(), text: m.chat_starter_song_text() },
  { label: m.chat_starter_automation_label(), text: m.chat_starter_automation_text() },
  { label: m.chat_starter_reminder(), text: m.chat_starter_reminder() },
];

// ---------- 用量与费用（usage.jsonl，见 apps/desktop/src-tauri/src/usage.rs） ----------

export interface UsageTotals {
  requests: number; inputTokens: number; cacheHitTokens: number; outputTokens: number;
  characters: number; audioMs: number; images: number;
  hitRate: number | null; cost: Record<string, number>; unpriced: number;
}
export interface UsageSummary {
  today: UsageTotals; month: UsageTotals;
  byKind: { kind: string; label: string; totals: UsageTotals }[];
  byModel: { model: string; totals: UsageTotals }[];
  /** 最近 60 天，每天的合计、按用途、按模型（旧的在前）。 */
  days: { day: string; totals: UsageTotals; byKind: { kind: string; label: string; totals: UsageTotals }[]; byModel: { model: string; totals: UsageTotals }[] }[];
  priceSource: string; note: string; ledger: string | null;
}

/** 一段对话的费用按类别（ADR 029 / 玩家要求：语音合成与识别、读屏、联网搜索也记到对话上）。 */
export type CostCategory = "chat" | "voice" | "screen" | "search" | "background" | "other";
export interface CategoryTotals { category: CostCategory; label: string; totals: UsageTotals }
export interface ConversationCosts {
  conversationId: string;
  /** 整段对话按类别。 */
  total: CategoryTotals[];
  /** 每一轮（turn = 那一轮玩家那句话的时间 atMs）按类别；模型调用不分轮（每轮的模型用量在消息里）。 */
  turns: { turn: number; byCategory: CategoryTotals[] }[];
}

const emptyTotals = (): UsageTotals => ({ requests: 0, inputTokens: 0, cacheHitTokens: 0, outputTokens: 0, characters: 0, audioMs: 0, images: 0, hitRate: null, cost: {}, unpriced: 0 });

/** 一段对话的费用（读本机账本 usage.jsonl；sinceMs = 对话开始的时间）。读不到时为 null。 */
export async function usageConversation(id: string, sinceMs: number): Promise<ConversationCosts | null> {
  if (isTauri) return invoke<ConversationCosts | null>("usage_conversation", { id, sinceMs });
  if (id !== previewConversation.id) return null;
  const voice: UsageTotals = { ...emptyTotals(), requests: 2, characters: 60, audioMs: 4_000, cost: { CNY: 0.0094 } };
  const screen: UsageTotals = { ...emptyTotals(), requests: 1, images: 1, cost: { CNY: 0.0015 } };
  return {
    conversationId: id,
    // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的数据），不是界面文字
    total: [{ category: "voice", label: "语音", totals: voice }, { category: "screen", label: "读屏", totals: screen }],
    turns: [{ turn: previewConversation.messages[0].atMs, byCategory: [{ category: "voice", label: "语音", totals: voice }, { category: "screen", label: "读屏", totals: screen }] }],
  };
}

export async function usageSummary(): Promise<UsageSummary> {
  if (isTauri) return invoke("usage_summary");
  const today: UsageTotals = { ...emptyTotals(), requests: 12, inputTokens: 182_000, cacheHitTokens: 171_000, outputTokens: 2_400, characters: 640, hitRate: 171_000 / 182_000, cost: { CNY: 0.0412 } };
  return {
    today, month: { ...today, requests: 230, inputTokens: 3_600_000, cacheHitTokens: 3_350_000, outputTokens: 51_000, characters: 9_800, audioMs: 420_000, images: 35, hitRate: 3_350_000 / 3_600_000, cost: { CNY: 0.92 } },
    byKind: [{ kind: "chat", label: "对话", totals: today }],
    byModel: [{ model: "deepseek/deepseek-flash", totals: today }],
    days: [3, 2, 1, 0].map((ago) => {
      const day = new Date(Date.now() - ago * 86_400_000).toISOString().slice(0, 10);
      const totals = { ...today, requests: 10 + ago, cost: { CNY: 0.02 + ago * 0.01 } };
      return { day, totals, byKind: [{ kind: "chat", label: "对话", totals }], byModel: [{ model: "deepseek/deepseek-flash", totals }] };
    }),
    priceSource: "界面预览", note: "界面预览的示例数字", ledger: null,
    // i18n-ignore-end
  };
}
