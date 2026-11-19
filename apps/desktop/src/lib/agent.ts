// 内置琴音 Agent（ADR 014 / 015）：对话、事件与权限询问。
// 事件按 ACP 的会话更新设计（消息片段、思考片段、工具调用及状态、权限请求）。
// 后端：apps/desktop/src-tauri/src/agent.rs。浏览器预览只展示示例对话，不调用任何 AI。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";

export interface ToolCall { id: string; name: string; arguments: string }
/** `cachedInputTokens`：输入里命中服务商前缀缓存的部分；`cost` + `currency`：按 Kotone 价格表估的费用（DeepSeek 用人民币）。 */
export interface TokenUsage { inputTokens?: number; outputTokens?: number; cachedInputTokens?: number; reasoningTokens?: number; costUsd?: number; cost?: number; currency?: string }
export type Attachment =
  | { kind: "data"; label: string; text: string }
  | { kind: "image"; label: string; mime: string; data?: string };

export type AgentMessage =
  | { role: "user"; text: string; attachments?: Attachment[]; source?: string; atMs: number; context?: string }
  | { role: "assistant"; text: string; toolCalls?: ToolCall[]; reasoning?: string; usage?: TokenUsage; atMs: number }
  | { role: "tool"; callId: string; name: string; content: string; isError: boolean; untrusted: boolean; atMs: number }
  /** `compaction`：上下文压缩的检查点（摘要）；它之前的消息不再发给模型。 */
  /** `recap`：对局复盘的正文（`text` 是标题），只给玩家看。 */
  | { role: "notice"; text: string; atMs: number; compaction?: string; usage?: TokenUsage; recap?: string };

/** `parent` / `label`：琴音派出的子会话（ADR 022），只能查看。 */
export interface Conversation { version: number; id: string; title: string; createdAtMs: number; updatedAtMs: number; messages: AgentMessage[]; parent?: string | null; label?: string | null }
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

export const PREVIEW_ERROR = "仅预览：浏览器里不会真的发出去，请在桌面应用里聊天。";

// ---------- 浏览器预览数据（只在 isTauri = false 时使用） ----------

const t0 = Date.now() - 5 * 60_000;
const previewConversation: Conversation = {
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
  ],
};
const preview = {
  current: previewConversation.id as string | null,
  conversations: [previewConversation],
  permissions: { mode: "default", rules: {} } as Permissions,
  hidden: [] as string[],
};
/** 浏览器预览的工具清单（与后端分组、默认规则一致的示例）。 */
const previewTools: Omit<AgentTool, "rule">[] = [
  { name: "reply.say", group: "game", description: "用语音或弹幕告诉玩家一句话", sideEffects: [], defaultRule: "allow", core: true },
  { name: "reminder.create", group: "game", description: "过一会儿提醒玩家", sideEffects: [], defaultRule: "allow", core: true },
  { name: "game_chat.send", group: "game", description: "把一句话打进游戏聊天框并发出", sideEffects: ["input"], defaultRule: "ask", core: true },
  { name: "companion.read_chat", group: "game", description: "截取游戏窗口，识别聊天记录", sideEffects: [], defaultRule: "allow", core: true },
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

export async function subscribeAgent(callback: (conversationId: string, event: AgentEvent) => void): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  return listen<{ conversationId: string; event: AgentEvent }>("kotone://agent-event", (e) => callback(e.payload.conversationId, e.payload.event));
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
  "tools.find": "查找工具",
  "reply.say": "播报",
  "reminder.create": "设提醒",
  "web.search": "联网搜索",
  "web.fetch": "读网页",
  "game_chat.send": "发到游戏聊天",
  "companion.read_chat": "看游戏聊天",
  "window.active": "看当前窗口",
  "window.list": "查看窗口",
  "game.list": "查看游戏",
  "game.focus": "切换到游戏",
  "game.launch": "打开游戏",
  "game.close": "关闭游戏",
  "game.look": "看游戏画面",
  "game.act": "操作游戏",
  "automation.list": "查看已保存的自动化",
  "automation.get": "查看自动化",
  "automation.run": "运行自动化",
  "automation.runs": "查看执行记录",
  "automation.stop": "停止自动化",
  "automation.save": "保存自动化",
  "automation.delete": "删除自动化",
  "settings.get": "读取设置",
  "settings.update": "修改设置",
  "ocr.recognize": "识别屏幕文字",
  "settings.describe": "查设置说明",
  "game.installed": "查装了哪些游戏",
  "models.list": "查看模型",
  "models.download": "下载模型",
  "models.activate": "换模型",
  "models.delete": "删除模型",
  "speech.model.download": "下载语音模型",
  "connections.list": "查看 API 连接",
  "connections.presets": "查看服务商",
  "connections.request_key": "请你粘贴 Key",
  "wake.status": "查看唤醒词",
  "wake.model.download": "下载唤醒模型",
  "runtime.status": "查看运行状态",
  "runtime.start": "启动语音识别",
  "app.restart": "重启 Kotone",
  "voice_chat.status": "查看游戏语音",
  "voice_chat.install": "安装虚拟麦克风",
  "voice_chat.default_mic": "改 Windows 默认麦克风",
  "voice_chat.say": "对队友说话",
  "voice_chat.test": "测试游戏语音",
  "memory": "记下来",
  "skill.view": "看技能",
  "skill.manage": "写技能",
  "soul.write": "改性格",
  "session.spawn": "派出子会话",
  "session.send": "给会话发消息",
  "session.list": "查看子会话",
  "session.history": "看子会话记录",
  "session.stop": "结束子会话",
  "voice.listen.start": "开始听",
  "voice.listen.stop": "不听了",
  "ask": "问你",
};
export function toolLabel(name: string): string {
  if (name.startsWith("saved.")) return "运行已保存的自动化";
  return toolLabels[name] ?? name;
}
/** 工具分组的中文名（agent_tools 的 group；没有的显示原名）。 */
export const toolGroupLabels: Record<string, string> = {
  game: "游戏与播报",
  automation: "自动化",
  screen: "屏幕与窗口",
  settings: "设置",
  models: "模型",
  meta: "查找工具",
  app: "其它",
};
export function toolGroupLabel(group: string): string {
  return toolGroupLabels[group] ?? group;
}

export function stopText(stop: StopReason): string | null {
  switch (stop.kind) {
    case "completed": return null;
    case "truncated": return "回复太长被截断了";
    case "step-limit": return "步骤太多，已停下";
    case "time-limit": return "用时太长，已停下";
    case "cancelled": return "已停止";
    case "failed": return `出错了：${stop.message}`;
  }
}

/** 用户消息的来源说明；聊天窗口里打的字不显示。 */
export function sourceLabel(source: string | undefined): string | null {
  switch (source) {
    case undefined: case "": case "chat": return null;
    case "assistant-text": return "文字入口";
    case "voice": return "游戏内 · 语音";
    case "quick-command": return "游戏内 · 快捷指令";
    case "tool": return "工具提交";
    case "automation": return "自动化结果";
    default: return source;
  }
}

/** 起手提示（ADR 014“已确认”）。 */
/** 聊天页的起手提示（ADR 028）。以冒号结尾的放进输入框让玩家接着写，其余直接发出。 */
export const starterPrompts = [
  { label: "你能做什么？", text: "你能做什么？" },
  { label: "帮我配置 Kotone", text: "帮我把 Kotone 配置好：先问我几个问题，再一项项帮我设置。" },
  { label: "这个版本什么英雄强？", text: "这个版本什么英雄强？" },
  { label: "做一个自动化", text: "帮我做一个自动化：" },
  { label: "5 分钟后提醒我", text: "5 分钟后提醒我" },
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

const emptyTotals = (): UsageTotals => ({ requests: 0, inputTokens: 0, cacheHitTokens: 0, outputTokens: 0, characters: 0, audioMs: 0, images: 0, hitRate: null, cost: {}, unpriced: 0 });

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
  };
}
