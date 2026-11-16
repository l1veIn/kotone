// 内置琴音 Agent（ADR 014 / 015）：对话、事件与权限询问。
// 事件按 ACP 的会话更新设计（消息片段、思考片段、工具调用及状态、权限请求）。
// 浏览器预览只展示示例对话，不调用任何 AI。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";

export interface ToolCall { id: string; name: string; arguments: string }
export interface TokenUsage { inputTokens?: number; outputTokens?: number; cachedInputTokens?: number; reasoningTokens?: number; costUsd?: number }
export type Attachment =
  | { kind: "data"; label: string; text: string }
  | { kind: "image"; label: string; mime: string; data?: string };

export type AgentMessage =
  | { role: "user"; text: string; attachments?: Attachment[]; source?: string; atMs: number }
  | { role: "assistant"; text: string; toolCalls?: ToolCall[]; reasoning?: string; usage?: TokenUsage; atMs: number }
  | { role: "tool"; callId: string; name: string; content: string; isError: boolean; untrusted: boolean; atMs: number }
  | { role: "notice"; text: string; atMs: number };

export interface Conversation { version: number; id: string; title: string; createdAtMs: number; updatedAtMs: number; messages: AgentMessage[] }
export interface ConversationSummary { id: string; title: string; createdAtMs: number; updatedAtMs: number; messageCount: number }

export type StopReason = { kind: "completed" | "truncated" | "step-limit" | "time-limit" | "cancelled" } | { kind: "failed"; message: string };
export type ToolStatus = "pending" | "in_progress" | "completed" | "failed";

export type AgentEvent =
  | { kind: "step"; step: number }
  | { kind: "chunk"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "toolCall"; call: ToolCall }
  | { kind: "toolCallUpdate"; callId: string; status: ToolStatus; content: string | null }
  | { kind: "appended"; message: AgentMessage }
  | { kind: "finished"; stop: StopReason; acpStopReason: string | null }
  | { kind: "idle" };

export interface PermissionRequest { callId: string; tool: string; title: string; reason: string; arguments: unknown }
export interface AgentAsk { askId: string; conversationId: string; request: PermissionRequest }
export type Answer = "allow-once" | "allow-for-conversation" | "deny";

export type PermissionMode = "default" | "allow-all" | "read-only";
export interface AgentState {
  current: string | null;
  running: string | null;
  conversations: ConversationSummary[];
  notReadyReason: string | null;
  permissions: { mode: PermissionMode; rules: Record<string, "allow" | "ask" | "deny"> };
}

export const PREVIEW_ERROR = "浏览器仅预览界面；请在桌面应用中和琴音聊天。";

// ---------- 浏览器预览数据 ----------

const t0 = Date.now() - 5 * 60_000;
const previewConversation: Conversation = {
  version: 1, id: "c-preview", title: "帮我做一个每秒按 Q 的自动化", createdAtMs: t0, updatedAtMs: t0 + 60_000,
  messages: [
    { role: "user", text: "帮我做一个每秒按 Q 的自动化", source: "chat", atMs: t0 },
    { role: "assistant", text: "好的，我先试运行一下，确认没问题再帮你保存。", atMs: t0 + 2_000,
      toolCalls: [{ id: "call-1", name: "automation.run", arguments: '{"name":"连按Q","source":"every(1000, function() key(\\"Q\\") end)"}' }],
      usage: { inputTokens: 1830, outputTokens: 96 } },
    { role: "tool", callId: "call-1", name: "automation.run", content: '{"status":"started","id":"run-7","target":"英雄联盟 (League of Legends.exe)"}', isError: false, untrusted: false, atMs: t0 + 9_000 },
    { role: "assistant", text: "已经开始了：每秒按一次 Q，按 Ctrl+Shift+F12 可以随时停止。要我把它保存下来吗？", atMs: t0 + 11_000 },
  ],
};

// ---------- 命令 ----------

export async function agentState(): Promise<AgentState> {
  if (!isTauri) {
    return {
      current: previewConversation.id, running: null, notReadyReason: null,
      conversations: [{ id: previewConversation.id, title: previewConversation.title, createdAtMs: t0, updatedAtMs: t0 + 60_000, messageCount: previewConversation.messages.length }],
      permissions: { mode: "default", rules: {} },
    };
  }
  return invoke("agent_state");
}
export async function agentConversation(id: string): Promise<Conversation | null> {
  if (!isTauri) return id === previewConversation.id ? structuredClone(previewConversation) : null;
  return invoke("agent_conversation", { id });
}
export async function agentSend(text: string): Promise<{ status: "started" | "steered"; conversationId: string }> {
  if (!isTauri) throw new Error(PREVIEW_ERROR);
  return invoke("agent_send", { text });
}
export async function agentStop(): Promise<boolean> {
  return isTauri ? invoke("agent_stop") : false;
}
export async function agentAnswer(askId: string, answer: Answer): Promise<boolean> {
  return isTauri ? invoke("agent_answer", { askId, answer }) : false;
}
export async function agentNewConversation(): Promise<void> {
  if (isTauri) await invoke("agent_new_conversation");
}
export async function agentSelectConversation(id: string): Promise<void> {
  if (isTauri) await invoke("agent_select_conversation", { id });
}
export async function agentDeleteConversation(id: string): Promise<boolean> {
  return isTauri ? invoke("agent_delete_conversation", { id }) : false;
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

// ---------- 展示 ----------

/** 工具名的中文说明（卡片标题用；没有的显示原名）。 */
const toolLabels: Record<string, string> = {
  "tools.find": "查找工具",
  "reply.say": "播报",
  "reminder.create": "设提醒",
  "game_chat.send": "发到游戏聊天",
  "companion.read_chat": "看游戏聊天",
  "window.active": "看当前窗口",
  "automation.list": "查看已保存的自动化",
  "automation.get": "查看自动化",
  "automation.run": "运行自动化",
  "automation.runs": "查看执行记录",
  "automation.stop": "停止自动化",
  "automation.save": "保存自动化",
  "automation.delete": "删除自动化",
  "settings.get": "读取设置",
  "settings.update": "修改设置",
};
export function toolLabel(name: string): string {
  if (name.startsWith("saved.")) return "运行已保存的自动化";
  return toolLabels[name] ?? name;
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

/** 起手提示（ADR 014“已确认”）。 */
export const starterPrompts = [
  { label: "做一个自动化", text: "帮我做一个自动化：" },
  { label: "看看队友在说什么", text: "看看队友在聊天里说了什么" },
  { label: "设个提醒", text: "5 分钟后提醒我" },
  { label: "我有哪些自动化", text: "我保存了哪些自动化？" },
];
