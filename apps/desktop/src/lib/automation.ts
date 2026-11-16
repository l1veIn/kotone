// 自动化（ADR 013）：执行记录（含提醒）、已保存的自动化、运行与停止。
// 浏览器预览使用本地示例数据，不运行任何程序、不按键。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import type { DeliveryKind } from "./assistant";

export type RunStatus = "pending" | "running" | "finished" | "failed" | "cancelled" | "missed";

export interface RunTarget { handle: number; pid: number; label: string }

export type RunAction =
  | { kind: "speak"; text: string; delivery: DeliveryKind[] }
  | {
    kind: "program"; name: string; source?: string; automationId?: string | null;
    args?: Record<string, unknown>; target?: RunTarget | null;
  };

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

export interface Automation {
  id: string;
  name: string;
  description: string;
  params: ParamSpec[];
  action: AutomationBody;
  createdBy: "user" | "companion" | "agent";
  createdAtMs: number;
  updatedAtMs: number;
}

export interface AutomationDraft {
  id?: string | null;
  name: string;
  description: string;
  params: ParamSpec[];
  action: AutomationBody;
}

export const PARAM_TYPES: { id: ParamType; label: string }[] = [
  { id: "number", label: "数字" },
  { id: "text", label: "文字" },
  { id: "boolean", label: "开关" },
  { id: "choice", label: "选项" },
  { id: "key", label: "按键" },
  { id: "region", label: "屏幕区域" },
  { id: "color", label: "颜色" },
  { id: "image", label: "图片" },
];

export function runTitle(run: AutomationRun): string {
  return run.action.kind === "speak" ? run.action.text : run.action.name || "自动化";
}

// ---------- 浏览器预览数据 ----------

const t0 = Date.now();
let previewRuns: AutomationRun[] = [
  {
    id: "preview-run-1", sessionId: "preview", status: "running", createdAtMs: t0 - 20_000, dueAtMs: t0 - 20_000, startedAtMs: t0 - 20_000,
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
  },
];
const previewListeners = new Set<() => void>();
const notify = () => previewListeners.forEach((listener) => listener());
const PREVIEW_ONLY = "浏览器仅预览界面；请在桌面应用中运行自动化。";

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
  throw new Error(PREVIEW_ONLY);
}

export async function subscribeAutomations(callback: () => void): Promise<UnlistenFn> {
  if (isTauri) return listen("kotone://automations-updated", callback);
  previewListeners.add(callback);
  return () => { previewListeners.delete(callback); };
}
