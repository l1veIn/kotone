// 原神插件（ADR 047 第 3 期）：驱动玩家装的 BetterGI。后端 plugins/genshin/src/lib.rs；工具 `genshin.*`：
// list（一条龙配置、配置组、计划）、status（全份在 structuredContent）、run、stop（如实说没有停止接口）、只给页面的 webhook（一键填好）。
// 设置在 `plugins.genshin.settings.path`（BetterGI 的目录；空 = 自己找）。插件关着时工具只回 `{status: "disabled"}`。
import { updateSettings } from "$lib/ipc";
import { pluginCall, type PluginToolResult } from "$lib/plugins";
import { settingsStore } from "$lib/stores/ui";
import type { Settings } from "$lib/ipc/contracts";

export const PLUGIN_ID = "genshin";
export const PAGE_HOME = "ui://kotone/genshin/home";
/** 推送“BetterGI 的运行”的卡片（后端 plugins/genshin/src/runs.rs）。 */
export const PAGE_RUN = "ui://kotone/genshin/run";

export type RunKind = "dragon" | "group" | "plan" | "task";
export type RunStatus = "starting" | "running" | "succeeded" | "partial" | "cancelled" | "failed" | "unknown";

/** 一次运行（卡片的数据，和后端的 `Run` 一致）。 */
export interface GenshinRun {
  key: string;
  kind: RunKind;
  name: string;
  status: RunStatus;
  message: string;
  step: string;
  error: string;
  byKotone: boolean;
  startedAtMs: number;
  finishedAtMs: number | null;
  planId?: string;
  pending?: string[];
}

export interface GenshinLists {
  status: string;
  text?: string;
  oneDragon: { name: string; selected: boolean }[];
  groups: { name: string; projects: number }[];
  plans: { id: string; name: string; description: string }[];
  plansSupported: boolean;
}

export interface PlanRun {
  planId?: string;
  planName: string;
  status: string;
  message: string;
  submittedAt: string;
  finishedAt: string;
}

export interface GenshinStatus {
  status: string;
  text?: string;
  searched?: string[];
  install?: { dir: string; exe: string; version: string | null; found: "settings" | "env" | "common" | "shortcut" };
  running?: boolean;
  webhook?: {
    url: string | null; listening: boolean; error: string; enabled: boolean; configured: boolean; toKotone: boolean;
    lastNoticeAtMs: number | null; lastTestAtMs: number | null;
  };
  external?: { httpApi: boolean; webSocket: boolean; mcp: boolean; port: number } | null;
  health?: { status?: string; uptimeSeconds?: number } | null;
  plans?: { supported: boolean; queue: { active: PlanRun[]; pending: number }; recent: PlanRun[] };
  current?: GenshinRun[];
  recent?: GenshinRun[];
  log?: { time: string; level: string; text: string }[];
  stop?: string;
  automation?: { allowed: boolean; text?: string };
}

/** 工具的回答：插件关着、数据开关关着时 null。 */
function body<T>(result: PluginToolResult, structured = false): T | null {
  const content = (structured ? (result.structuredContent ?? result.content) : result.content) as (T & { status?: string }) | null;
  if (!content || content.status === "disabled") return null;
  return content;
}

export async function genshinStatus(): Promise<GenshinStatus | null> {
  const result = await pluginCall(PLUGIN_ID, "genshin.status");
  const short = result.content as { status?: string } | null;
  if (!short || short.status === "disabled") return null;
  return short.status === "ok" ? body<GenshinStatus>(result, true) : (short as GenshinStatus);
}

export async function genshinLists(): Promise<GenshinLists | null> {
  return body<GenshinLists>(await pluginCall(PLUGIN_ID, "genshin.list"));
}

export interface RunAnswer { status: string; text?: string; name?: string; notes?: string[] }

export async function genshinRun(kind: "dragon" | "group" | "plan", name: string): Promise<RunAnswer> {
  const args: Record<string, unknown> = { kind };
  if (name) args.name = name;
  return (await pluginCall(PLUGIN_ID, "genshin.run", args)).content as RunAnswer;
}

export async function genshinFillWebhook(): Promise<{ status: string; text?: string; url?: string; backup?: string }> {
  return (await pluginCall(PLUGIN_ID, "genshin.webhook", { action: "fill" })).content as { status: string; text?: string };
}

/** 设置里填的 BetterGI 路径（空 = 自己找）。 */
export function genshinPath(settings: Settings | null | undefined): string {
  const raw = (settings?.plugins?.[PLUGIN_ID]?.settings ?? {}) as Record<string, unknown>;
  return typeof raw.path === "string" ? raw.path : "";
}

export async function saveGenshinPath(path: string): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [PLUGIN_ID]: { settings: { path: path.trim() } } } }));
}

export const isRun = (data: unknown): data is GenshinRun =>
  !!data && typeof data === "object" && "kind" in data && "status" in data && "startedAtMs" in data;
