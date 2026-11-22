// 自动化（ADR 013）：执行记录（含提醒）、已保存的自动化、运行与停止。
// 浏览器预览使用本地示例数据，不运行任何程序、不按键。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import type { DeliveryKind } from "./assistant";
import { m } from "./paraglide/messages.js";

export type RunStatus = "pending" | "running" | "finished" | "failed" | "cancelled" | "missed";

export interface RunTarget { handle: number; pid: number; label: string }

export type RunAction =
  | { kind: "speak"; text: string; delivery: DeliveryKind[] }
  | {
    kind: "program"; name: string; source?: string; automationId?: string | null;
    args?: Record<string, unknown>; target?: RunTarget | null;
  }
  /** 琴音替玩家打进游戏聊天框的一句话（只作记录） */
  | { kind: "chat"; text: string; sent: boolean; profile?: string };

export interface ProgramReport { outcome: "finished" | "stopped" | "limit" | "error"; keys: number; durationMs: number; log: string[] }

/** 一次执行（执行记录） */
export interface AutomationRun {
  id: string;
  sessionId: string;
  action: RunAction;
  createdAtMs: number;
  dueAtMs: number;
  status: RunStatus;
  message?: string | null;
  startedAtMs?: number | null;
  finishedAtMs?: number | null;
  report?: ProgramReport | null;
}

export interface RunsSnapshot { running: number; pending: number; maxRunning: number; runs: AutomationRun[] }

export type ParamType = "number" | "text" | "boolean" | "choice" | "key" | "region" | "color" | "image";
export interface Region { x: number; y: number; w: number; h: number }
export interface Color { r: number; g: number; b: number }

/** 入参声明（没有 default = 必填） */
export interface ParamSpec {
  name: string;
  label: string;
  description: string;
  type: ParamType;
  min?: number | null;
  max?: number | null;
  integer?: boolean;
  maxLength?: number;
  options?: string[];
  default?: unknown;
}

export type AutomationBody =
  | { kind: "program"; source?: string }
  | { kind: "speak"; text: string; delivery: DeliveryKind[] };

/** 定时（类似 crontab，本地时间）：到点用 args 运行一次；错过的不补。 */
export interface Schedule { cron: string; enabled: boolean; args: Record<string, unknown> }

export interface Automation {
  id: string;
  name: string;
  description: string;
  params: ParamSpec[];
  action: AutomationBody;
  createdBy: "user" | "companion" | "agent";
  createdAtMs: number;
  updatedAtMs: number;
  schedule?: Schedule | null;
  /** 定时的下一次（列表里才有）。 */
  nextRunAtMs?: number | null;
}

/** 内置的“X 秒之后提醒我”。 */
export const BUILTIN_REMIND_ID = "builtin-remind-later";

/** 定时的常用写法（界面里选）；其余写 cron。 */
export type SchedulePreset =
  | { kind: "daily"; time: string }
  | { kind: "weekdays"; time: string }
  | { kind: "hourly" }
  | { kind: "minutes"; every: number }
  | { kind: "cron"; cron: string };

export function presetToCron(preset: SchedulePreset): string {
  const hm = (time: string) => {
    const [h, m] = time.split(":").map((v) => Number(v) || 0);
    return `${Math.min(59, Math.max(0, m))} ${Math.min(23, Math.max(0, h))}`;
  };
  switch (preset.kind) {
    case "daily": return `${hm(preset.time)} * * *`;
    case "weekdays": return `${hm(preset.time)} * * 1-5`;
    case "hourly": return "0 * * * *";
    case "minutes": return `*/${Math.min(59, Math.max(1, Math.round(preset.every)))} * * * *`;
    case "cron": return preset.cron.trim();
  }
}

export function cronToPreset(cron: string): SchedulePreset {
  const t = cron.trim();
  const pad = (n: string) => n.padStart(2, "0");
  let m = /^(\d{1,2}) (\d{1,2}) \* \* \*$/.exec(t);
  if (m) return { kind: "daily", time: `${pad(m[2])}:${pad(m[1])}` };
  m = /^(\d{1,2}) (\d{1,2}) \* \* 1-5$/.exec(t);
  if (m) return { kind: "weekdays", time: `${pad(m[2])}:${pad(m[1])}` };
  if (t === "0 * * * *" || t === "@hourly") return { kind: "hourly" };
  m = /^\*\/(\d{1,2}) \* \* \* \*$/.exec(t);
  if (m) return { kind: "minutes", every: Number(m[1]) };
  return { kind: "cron", cron: t };
}

/** 给人看的定时说明：“每天 08:00”“工作日 09:30”“每 10 分钟”…… */
export function describeSchedule(cron: string): string {
  const p = cronToPreset(cron);
  switch (p.kind) {
    case "daily": return m.auto_schedule_daily({ time: p.time });
    case "weekdays": return m.auto_schedule_weekdays({ time: p.time });
    case "hourly": return m.auto_schedule_hourly();
    case "minutes": return m.auto_schedule_every_minutes({ minutes: p.every });
    case "cron": return cron === "@daily" ? m.auto_schedule_daily_midnight() : m.auto_schedule_cron({ cron });
  }
}

export interface AutomationDraft {
  id?: string | null;
  name: string;
  description: string;
  params: ParamSpec[];
  action: AutomationBody;
  schedule?: Schedule | null;
}

export const PARAM_TYPES: { id: ParamType; label: string }[] = [
  { id: "number", label: m.auto_param_type_number() },
  { id: "text", label: m.auto_param_type_text() },
  { id: "boolean", label: m.auto_param_type_boolean() },
  { id: "choice", label: m.auto_param_type_choice() },
  { id: "key", label: m.auto_param_type_key() },
  { id: "region", label: m.auto_param_type_region() },
  { id: "color", label: m.auto_param_type_color() },
  { id: "image", label: m.auto_param_type_image() },
];

export function runTitle(run: AutomationRun): string {
  if (run.action.kind === "program") return run.action.name || m.auto_kind_program();
  return run.action.text;
}

// ---------- 浏览器预览数据 ----------

const t0 = Date.now();
/** 预览数据：今天或明天的 h:m。 */
function nextAt(h: number, m: number): number {
  const d = new Date();
  d.setHours(h, m, 0, 0);
  if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
  return d.getTime();
}
let previewRuns: AutomationRun[] = [
  {
    id: "preview-run-1", sessionId: "preview", status: "running", createdAtMs: t0 - 20_000, dueAtMs: t0 - 20_000, startedAtMs: t0 - 20_000,
    // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的数据），不是界面文字
    action: { kind: "program", name: "界面示例：每秒按 Q", automationId: "preview-auto-1", args: { key: "Q", interval: 1000 }, target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } },
  },
  {
    id: "preview-run-2", sessionId: "preview", status: "pending", createdAtMs: t0, dueAtMs: t0 + 300_000,
    action: { kind: "speak", text: "界面示例：打野有闪了", delivery: ["voice", "danmaku"] },
  },
  {
    id: "preview-run-3", sessionId: "preview", status: "finished", createdAtMs: t0 - 600_000, dueAtMs: t0 - 600_000, startedAtMs: t0 - 600_000, finishedAtMs: t0 - 590_000,
    action: { kind: "program", name: "界面示例：变黑就按回车", target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } },
    report: { outcome: "finished", keys: 1, durationMs: 9_800, log: ["左上角变黑了"] },
  },
];
let previewAutomations: Automation[] = [
  {
    id: "preview-auto-1", name: "界面示例：连按某键", description: "每隔一段时间按一次指定的键", createdBy: "user", createdAtMs: t0, updatedAtMs: t0,
    params: [
      { name: "key", label: "按键", description: "", type: "key", default: "Q" },
      { name: "interval", label: "间隔（毫秒）", description: "", type: "number", min: 50, max: 10000, integer: true, default: 1000 },
      { name: "times", label: "次数", description: "", type: "number", min: 1, max: 1000, integer: true },
    ],
    action: { kind: "program", source: "every(args.interval, function() key(args.key) end, { times = args.times })" },
  },
  {
    id: "preview-auto-2", name: "界面示例：喝水提醒", description: "", createdBy: "agent", createdAtMs: t0, updatedAtMs: t0,
    params: [], action: { kind: "speak", text: "该喝水了", delivery: ["danmaku"] },
    schedule: { cron: "0 22 * * *", enabled: true, args: {} }, nextRunAtMs: nextAt(22, 0),
  },
  {
    id: BUILTIN_REMIND_ID, name: "X 秒之后提醒我", description: "过一会儿用语音和弹幕提醒你一句话，例如 300 秒后提醒“打小龙”。",
    createdBy: "user", createdAtMs: t0, updatedAtMs: t0,
    params: [
      { name: "delay_seconds", label: "几秒之后", description: "最多一天（86400 秒）", type: "number", min: 1, max: 86400, integer: true, default: 60 },
      { name: "text", label: "提醒什么", description: "", type: "text", maxLength: 100 },
    ],
    action: { kind: "speak", text: "{text}", delivery: ["voice", "danmaku"] },
  },
];
const previewListeners = new Set<() => void>();
const notify = () => previewListeners.forEach((listener) => listener());
const PREVIEW_ONLY = m.auto_preview_only();

// ---------- 调用 ----------

export async function listRuns(): Promise<RunsSnapshot> {
  if (isTauri) return invoke("automation_runs");
  const runs = structuredClone(previewRuns);
  return {
    running: runs.filter((r) => r.status === "running" && r.action.kind === "program").length,
    pending: runs.filter((r) => r.status === "pending").length,
    maxRunning: 3,
    runs,
  };
}

/** 取消 / 停止一条执行；不给 id = 停止全部自动化程序（等待中的提醒保留）。返回涉及条数。 */
export async function stopRuns(id?: string): Promise<number> {
  if (isTauri) return invoke("automation_stop", { id: id ?? null });
  let count = 0;
  for (const run of previewRuns) {
    const open = run.status === "pending" || run.status === "running";
    const hit = id ? run.id === id : run.action.kind === "program";
    if (open && hit) {
      run.status = "cancelled";
      run.message = "用户停止";
      run.finishedAtMs = Date.now();
      count++;
    }
  }
  notify();
  return count;
}

export async function listAutomations(): Promise<Automation[]> {
  if (isTauri) return (await invoke<{ automations: Automation[] }>("automation_saved_list")).automations;
  return structuredClone(previewAutomations);
}

export async function getAutomation(id: string): Promise<Automation> {
  if (isTauri) return invoke("automation_get", { id });
  const found = previewAutomations.find((a) => a.id === id);
  if (!found) throw new Error(`找不到自动化 ${id}`);
  return structuredClone(found);
}

export async function saveAutomation(draft: AutomationDraft): Promise<Automation> {
  if (isTauri) return invoke("automation_save", { draft });
  // 草稿可能带着界面状态的代理：存成普通 JSON，之后 structuredClone 才不会出错
  draft = JSON.parse(JSON.stringify(draft)) as AutomationDraft;
  if (!draft.name.trim()) throw new Error("自动化名字不能为空，最多 40 字");
  const now = Date.now();
  const existing = draft.id ? previewAutomations.find((a) => a.id === draft.id) : undefined;
  const saved: Automation = {
    id: existing?.id ?? `preview-auto-${now}`, name: draft.name.trim(), description: draft.description.trim(),
    params: draft.params.map((p) => ({ ...p, label: p.label.trim() || p.name })), action: draft.action,
    createdBy: existing?.createdBy ?? "user", createdAtMs: existing?.createdAtMs ?? now, updatedAtMs: now,
    schedule: draft.schedule ?? null, nextRunAtMs: draft.schedule?.enabled ? now + 3_600_000 : null,
  };
  previewAutomations = existing ? previewAutomations.map((a) => (a.id === saved.id ? saved : a)) : [...previewAutomations, saved];
  notify();
  return structuredClone(saved);
}

export async function deleteAutomation(id: string): Promise<boolean> {
  if (isTauri) return invoke("automation_delete", { id });
  const before = previewAutomations.length;
  previewAutomations = previewAutomations.filter((a) => a.id !== id);
  notify();
  return previewAutomations.length !== before;
}

/** 从设置页运行：程序类倒数 switchSeconds 秒（切到游戏）后以那时的前台窗口为目标。 */
export async function runAutomation(id: string, args: Record<string, unknown>, switchSeconds: number): Promise<{ status: string; seconds?: number }> {
  if (isTauri) return invoke("automation_run_saved", { id, args, switchSeconds });
  // 浏览器预览：只在列表里加一条示例执行，不会真的按键或出声
  const automation = previewAutomations.find((a) => a.id === id);
  if (!automation) throw new Error(`找不到自动化 ${id}`);
  const now = Date.now();
  const missing = automation.params.find((p) => (p.default === undefined || p.default === null) && (args[p.name] === undefined || args[p.name] === ""));
  if (missing) throw new Error(`缺少参数 ${missing.label || missing.name}`);
  if (automation.action.kind === "speak") {
    const values = Object.fromEntries(automation.params.map((p) => [p.name, args[p.name] ?? p.default]));
    const text = automation.action.text.replace(/\{(\w+)\}/g, (_all, name: string) => String(values[name] ?? ""));
    const delay = Number(values.delay_seconds ?? 0) * 1000;
    previewRuns = [...previewRuns, { id: `preview-run-${now}`, sessionId: "ui", status: "pending", createdAtMs: now, dueAtMs: now + delay,
      action: { kind: "speak", text, delivery: automation.action.delivery } }];
    notify();
    return { status: "scheduled" };
  }
  previewRuns = [...previewRuns, { id: `preview-run-${now}`, sessionId: "ui", status: "running", createdAtMs: now, dueAtMs: now, startedAtMs: now,
    action: { kind: "program", name: automation.name, automationId: id, args, target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } } }];
    // i18n-ignore-end
  notify();
  return { status: "countdown", seconds: switchSeconds };
}

export async function subscribeAutomations(callback: () => void): Promise<UnlistenFn> {
  if (isTauri) return listen("kotone://automations-updated", callback);
  previewListeners.add(callback);
  return () => { previewListeners.delete(callback); };
}
