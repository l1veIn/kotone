// 自动化（ADR 013、ADR 045）：执行记录（含提醒）、已保存的自动化、运行与停止。动作只有程序一种：
// 说一句、弹幕、推送、交给她都是程序里的函数；“用不用游戏窗口”“会不会叫她”由后端看源码算好给界面。
// 浏览器预览使用本地示例数据，不运行任何程序、不按键。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import type { DeliveryKind } from "./assistant";
import { m } from "./paraglide/messages.js";

export type RunStatus = "pending" | "running" | "finished" | "failed" | "cancelled" | "missed";

export interface RunTarget { handle: number; pid: number; label: string }

/** 一次执行跑的程序（快照）。`automationId`、`target` 总是在（临时程序、不用游戏窗口的是 null）；
 * 只有按游戏事件触发的才带 `event`（只露类型和游戏）。列表里带源码（认提醒用）。 */
export interface RunAction {
  kind: "program";
  name: string;
  source?: string;
  automationId: string | null;
  args: Record<string, unknown>;
  target: RunTarget | null;
  event?: { type: string; game?: string | null };
}

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

/** 自动化做什么：一段程序（列表里不带源码，`automation_get` 才带）。 */
export interface AutomationBody { kind: "program"; source?: string }

/** 游戏事件触发（ADR 036 D6）：哪些事发生时运行。 */
export interface EventTrigger {
  /** 事件类型（“或”），也可以写前缀 lol.* */
  event: string[];
  /** 只在这个游戏配置上触发；不写 = 任何游戏 */
  game?: string | null;
  /** 对事件数据的子集匹配，数组表示“或” */
  match?: Record<string, unknown>;
  /** 阶段保持多少秒才触发（只用于 game.phase_changed） */
  for?: number;
  mode?: "single" | "restart" | "queued";
  oncePerMatch?: boolean;
  cooldownSeconds?: number;
  enabled: boolean;
}

/** 程序里的 ask()（交给她）按游戏事件触发时每个自动化每局最多几次（和后端 `games::ASKS_PER_MATCH` 一致）。 */
export const ASKS_PER_MATCH = 3;

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
  /** 游戏事件触发。 */
  on?: EventTrigger | null;
  /** 定时的下一次（列表里才有）。 */
  nextRunAtMs?: number | null;
  /** 程序里用到游戏窗口（按键、看屏幕……）：▶ 先倒数让你切到游戏，运行时绑那个窗口（ADR 045 D4）。 */
  usesWindow: boolean;
  /** 程序里会叫她（ask）：每次花一次 AI 调用，不当工具交给她。 */
  asks: boolean;
  /** 程序里 `tool()` 调的工具（按出现的先后）现在怎么样（ADR 046 D9，后端按权限和插件开关算）。 */
  tools: ProgramTool[];
  /** 有要你在「自动化」页点“允许”的工具：允许之前它完全不运行（ADR 046 D3）。 */
  needsConfirm: boolean;
}

/** 程序里调的一个工具现在怎么样：ready 能调；confirm 要你允许；off 插件或数据开关关着；blocked 调不了。 */
export type ToolState = "ready" | "confirm" | "off" | "blocked";

/** `title` 是工具说明的第一句（可能很长、可能是英文），界面上用 `toolLabel`；`reason` 说为什么（ready 是空的）。 */
export interface ProgramTool { name: string; title: string; state: ToolState; reason: string }

/** 编辑器里“会调用：…”那一行（`automation_tools`，照编辑器里还没存的程序算）：工具名不是直接写的时 `tools` 为空、
 * `error` 说原因（这样存不进来）。 */
export interface ToolsPreview { tools: ProgramTool[]; needsConfirm: boolean; error: string | null }

const HAN = /[\u4e00-\u9fff]/;
/** 工具在界面上的名字（`title` 很长，例如“放歌（网易云音乐，Kotone 自己放，马上出声）”）：说明第一句截到第一个括号、
 * 逗号、冒号之前，短的中文写成“放歌（music）”，别的（英文说明、太长）只写工具名。 */
export function toolLabel(tool: Pick<ProgramTool, "name" | "title">): string {
  const short = tool.title.split(/[\uff08(\uff0c,\uff1a:\uff1b;\u3002]/)[0].trim();
  return short && short !== tool.name && HAN.test(short) && [...short].length <= 12 ? m.auto_tools_label({ title: short, name: tool.name }) : tool.name;
}

/** 一个工具为什么调不了 / 要允许的那句：原因里本来就写了工具名的照原样，没写的前面加上是哪个工具。 */
export function toolReason(tool: ProgramTool): string {
  return tool.reason.includes(tool.name) ? tool.reason : m.auto_tools_reason({ tool: toolLabel(tool), reason: tool.reason });
}

/** 插件关着、调不了的工具：存得下，改好之前它不运行（ADR 046 D3）。 */
export function unusableTools(tools: ProgramTool[]): ProgramTool[] {
  return tools.filter((t) => t.state === "off" || t.state === "blocked");
}

/** 新建时选的模板（ADR 045 D9）：说一句 / 推送一条 / 让她做 / 空白程序，和带触发的“一条龙跑完推一行”。名字和说明界面按 id 取文案。 */
export interface AutomationTemplate {
  id: string;
  name: string;
  description: string;
  params: ParamSpec[];
  source: string;
  /** 带好的“当……的时候”（没有 = 新建时自己选）。 */
  on?: EventTrigger;
  /** 要哪个第三方插件：Kotone 预设的 id（`zzz-onedragon`）；触发里的事件名以它开头，界面换成用着这份预设的插件的 id。 */
  preset?: string;
}

/** `automation_saved_list` 的结果。 */
export interface SavedList { automations: Automation[]; max: number; templates: AutomationTemplate[] }

/** ▶ 的结果：用到游戏窗口的先倒数；不用的直接排上（马上开始的是 started，晚一会儿的是 scheduled）。 */
export type RunSavedResult =
  | { status: "countdown"; seconds: number }
  | { status: "started" | "scheduled"; id: string; target: null; dueAtUnixMs: number };

/** 内置的“X 秒之后提醒我”。 */
export const BUILTIN_REMIND_ID = "builtin-remind-later";
/** 内置模板“英雄联盟进加载界面播报突破口”（默认关）。 */
export const BUILTIN_LOL_SCOUT_ID = "builtin-lol-scout-on-loading";
/** 内置的“英雄联盟这局打得特别好或特别差时她说几句”（按“这局结算了”触发，默认开，ADR 050）。 */
export const BUILTIN_LOL_GAME_END_ID = "builtin-lol-game-end-cheer";

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
  on?: EventTrigger | null;
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

/** 提醒的三段固定程序（和后端 `automation.rs` 的 REMIND_* 一致，ADR 045 D3）→ 回答方式。 */
const REMINDER_SOURCES: Record<string, DeliveryKind[]> = {
  "say(args.text)": ["voice"],
  "toast(args.text)": ["danmaku"],
  "say(args.text, { toast = true })": ["voice", "danmaku"],
};

/** 提醒形状的程序（三段固定源码之一、`args.text` 是文字）：她设的提醒、旧的播报、内置“X 秒之后提醒我”跑出来的都是。
 * 和后端 `Run::as_reminder` 同一个认法；执行记录里带源码，所以不靠“有没有绑窗口”猜。 */
export function reminderOf(run: AutomationRun): { text: string; delivery: DeliveryKind[] } | null {
  const delivery = REMINDER_SOURCES[run.action.source?.trim() ?? ""];
  const text = run.action.args?.text;
  return delivery && typeof text === "string" ? { text, delivery } : null;
}

/** 展示名：提醒取正文，别的取名字。 */
export function runTitle(run: AutomationRun): string {
  return reminderOf(run)?.text || run.action.name || m.auto_kind_program();
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

/** 预览里粗略地看程序用到了什么（真实应用由后端 `ProgramUses::of` 算）：去掉注释和字符串，找当函数名用的名字。 */
const WINDOW_FUNCTIONS = ["key", "key_down", "key_up", "type_text", "chat", "click", "mouse_move", "mouse_down", "mouse_up", "scroll",
  "brightness", "color_at", "ocr", "find_text", "find_color", "find_image", "game_focused"];
function previewUses(source: string): { usesWindow: boolean; asks: boolean } {
  const code = source.replace(/--\[(=*)\[[\s\S]*?\]\1\]|--[^\n]*|\[(=*)\[[\s\S]*?\]\2\]|"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'/g, " ");
  const names = new Set([...code.matchAll(/(?<![.:\w])([A-Za-z_]\w*)(?!\s*=[^=])/g)].map((match) => match[1]));
  return { usesWindow: WINDOW_FUNCTIONS.some((name) => names.has(name)), asks: names.has("ask") };
}

/** 预览里的工具（真实应用由后端按权限、插件开关和工具目录算，ADR 046 D2、D3）：标题是说明的第一句。 */
// i18n-ignore-start：预览里模拟后端给的工具说明和原因
const PREVIEW_TOOLS: Record<string, { title: string; state: ToolState; reason: string }> = {
  music: { title: "放歌（网易云音乐，Kotone 自己放，马上出声）", state: "ready", reason: "" },
  "lol.match": { title: "英雄联盟这局现在的样子（马上返回）：模式、游戏时间、比分", state: "ready", reason: "" },
  "settings.update": { title: "Deep-merge a partial settings patch, exactly like saving in the settings page: values are validated…", state: "confirm", reason: "会修改设置" },
  "game_chat.send": { title: "把一段话发到当前游戏的聊天", state: "blocked", reason: "game_chat.send 会往游戏里打字或按键，程序里不能调：用程序自己的 chat() / key() / click()" },
  "lol.scout": { title: "查这局 10 个人的战绩", state: "blocked", reason: "lol.scout 的结果是别人写的字，程序拿不到（只回 status），调了也白调" },
};
const previewMissing = (name: string) => /^(automation|saved)\./.test(name)
  ? `程序里不能调自动化自己的工具（${name}）：会绕圈，也能绕过确认`
  : `找不到工具 ${name}（插件没装或被删了，或者名字写错了）`;
const NOT_A_LITERAL = "tool() 的第一个参数要直接写工具名，例如 tool(\"music\", { action = \"play\" })：不能用变量或拼出来的名字，也别拿 tool 当变量名";
// i18n-ignore-end

/** 预览里粗略地列程序调的工具（真实应用由后端 `tools_called` 用记号器算）：去掉注释，`tool(` / `tool ` 后面要直接写名字。 */
function previewToolNames(source: string): { names: string[]; error: string | null } {
  const code = source.replace(/--\[(=*)\[[\s\S]*?\]\1\]|--[^\n]*/g, " ");
  const names: string[] = [];
  const call = /(?<![.:\w])tool\s*(\()?\s*/g;
  for (let found = call.exec(code); found; found = call.exec(code)) {
    const literal = /^(["'])((?:[^"'\\\n]|\\.)*)\1/.exec(code.slice(call.lastIndex));
    if (literal) {
      if (!names.includes(literal[2])) names.push(literal[2]);
    } else if (found[1]) {
      return { names: [], error: NOT_A_LITERAL };
    }
  }
  return { names, error: null };
}

/** 预览里每个工具现在怎么样：要允许的，`allowed` 里有它就算能调（程序没改过才传）。 */
function previewToolStates(source: string, allowed: string[]): ToolsPreview {
  const { names, error } = previewToolNames(source);
  const tools: ProgramTool[] = names.map((name) => {
    const known = PREVIEW_TOOLS[name];
    if (!known) return { name, title: name, state: "blocked", reason: previewMissing(name) };
    return known.state === "confirm" && allowed.includes(name)
      ? { name, title: known.title, state: "ready", reason: "" }
      : { name, title: known.title, state: known.state, reason: known.reason };
  });
  return { tools, needsConfirm: tools.some((t) => t.state === "confirm"), error };
}

const REMIND_BOTH = "say(args.text, { toast = true })";
let previewRuns: AutomationRun[] = [
  {
    id: "preview-run-1", sessionId: "preview", status: "running", createdAtMs: t0 - 20_000, dueAtMs: t0 - 20_000, startedAtMs: t0 - 20_000,
    // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的数据），不是界面文字
    action: { kind: "program", name: "界面示例：每秒按 Q", source: "every(args.interval, function() key(args.key) end, { times = args.times })", automationId: "preview-auto-1", args: { key: "Q", interval: 1000 }, target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } },
  },
  {
    // 她设的提醒：提醒形状的程序，不属于任何已保存的自动化、不绑窗口
    id: "preview-run-2", sessionId: "preview", status: "pending", createdAtMs: t0, dueAtMs: t0 + 300_000,
    action: { kind: "program", name: "界面示例：打野有闪了", source: REMIND_BOTH, automationId: null, args: { text: "界面示例：打野有闪了" }, target: null },
  },
  {
    id: "preview-run-3", sessionId: "preview", status: "finished", createdAtMs: t0 - 600_000, dueAtMs: t0 - 600_000, startedAtMs: t0 - 600_000, finishedAtMs: t0 - 590_000,
    action: { kind: "program", name: "界面示例：变黑就按回车", source: "wait_until(function() return brightness(region(0, 0, 0.1, 0.1)) < 0.1 end); key(\"Enter\")", automationId: null, args: {},
      target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } },
    report: { outcome: "finished", keys: 1, durationMs: 9_800, log: ["左上角变黑了"] },
  },
];
/** 预览里存着的自动化：调的工具和要不要允许每次照源码现算（`describePreview`）。 */
type PreviewAutomation = Omit<Automation, "tools" | "needsConfirm">;
let previewAutomations: PreviewAutomation[] = [
  {
    id: "preview-auto-1", name: "界面示例：连按某键", description: "每隔一段时间按一次指定的键", createdBy: "user", createdAtMs: t0, updatedAtMs: t0,
    params: [
      { name: "key", label: "按键", description: "", type: "key", default: "Q" },
      { name: "interval", label: "间隔（毫秒）", description: "", type: "number", min: 50, max: 10000, integer: true, default: 1000 },
      { name: "times", label: "次数", description: "", type: "number", min: 1, max: 1000, integer: true },
    ],
    action: { kind: "program", source: "every(args.interval, function() key(args.key) end, { times = args.times })" },
    usesWindow: true, asks: false,
  },
  {
    // 以前的“播报”转换过来就是这样一小段程序（ADR 045 D1）
    id: "preview-auto-2", name: "界面示例：喝水提醒", description: "", createdBy: "agent", createdAtMs: t0, updatedAtMs: t0,
    params: [], action: { kind: "program", source: "toast(\"该喝水了\")" },
    schedule: { cron: "0 22 * * *", enabled: true, args: {} }, nextRunAtMs: nextAt(22, 0),
    usesWindow: false, asks: false,
  },
  {
    id: BUILTIN_REMIND_ID, name: "X 秒之后提醒我", description: "过一会儿用语音和弹幕提醒你一句话，例如 300 秒后提醒“打小龙”。",
    createdBy: "user", createdAtMs: t0, updatedAtMs: t0,
    params: [
      { name: "delay_seconds", label: "几秒之后", description: "最多一天（86400 秒）", type: "number", min: 1, max: 86400, integer: true, default: 60 },
      { name: "text", label: "提醒什么", description: "", type: "text", maxLength: 100 },
    ],
    action: { kind: "program", source: REMIND_BOTH },
    usesWindow: false, asks: false,
  },
  {
    id: BUILTIN_LOL_SCOUT_ID, name: "英雄联盟进加载界面播报突破口",
    description: "进加载界面时她查一下这局 10 个人的战绩，用一句话告诉你对面谁是突破口。每次花一次 AI 调用，默认关。",
    createdBy: "user", createdAtMs: t0, updatedAtMs: t0, params: [],
    action: { kind: "program", source: "ask(\"这局进加载界面了。用 lol.scout 看看对面谁是突破口，用一句话告诉我原因（说成信息，不贴标签）。\")" },
    on: { event: ["game.phase_changed"], game: "lol", match: { to: "loading" }, oncePerMatch: true, enabled: false },
    usesWindow: false, asks: true,
  },
];
/** 和后端 `library::templates()` 一样的四个模板（名字和说明界面按 id 取文案，这里的只是后端给的原文）。 */
const previewTemplates: AutomationTemplate[] = [
  { id: "say", name: "说一句", description: "她开口说一句话。想同时弹幕就写 say(args.text, { toast = true })。",
    params: [{ name: "text", label: "说什么", description: "", type: "text", maxLength: 200, default: "该看小地图了" }], source: "say(args.text)\n" },
  { id: "push", name: "推送一条", description: "在 Kotone 聊天里记一条、悬浮窗显示一行；不出声，不调用 AI。最多 40 字。",
    params: [{ name: "text", label: "推送什么", description: "", type: "text", maxLength: 40, default: "记得买真眼" }], source: "push(args.text)\n" },
  { id: "ask", name: "让她做", description: "把一句话交给她，她看着办（查战绩、说话、调工具都行）。每次花一次 AI 调用；按游戏事件触发时每局最多 3 次。",
    params: [{ name: "request", label: "让她做什么", description: "", type: "text", maxLength: 200, default: "看看现在的情况，用一句话告诉我该注意什么" }], source: "ask(args.request)\n" },
  { id: "blank", name: "空白程序", description: "自己写一段程序。要通知就写 say / toast / push，不写就安静；按键类可以复制内置的程序。",
    params: [], source: "-- 在这里写程序，例如：\n-- say(\"该买眼了\")\n" },
  { id: "onedragon-push", name: "一条龙跑完推一行",
    description: "绝区零一条龙跑完或出错时，在 Kotone 聊天里记一行、悬浮窗显示一行；不出声，不调用 AI。要先加绝区零一条龙插件，并在它的「设置 → 事件」里打开“跑完了”“出错了”。",
    params: [{ name: "to", label: "现在的状态（事件自动填）", description: "", type: "text", maxLength: 20, default: "success" }],
    source: "-- 绝区零一条龙跑完 / 出错时推一行（args.to 由事件自动填上：success 或 failed）\nif args.to == \"failed\" then\n  push(\"绝区零一条龙出错了，去看看\")\nelse\n  push(\"绝区零一条龙跑完了\")\nend\n",
    on: { event: ["zzz-onedragon.run_finished", "zzz-onedragon.run_failed"], game: "zzz", match: {}, for: 0, mode: "single", oncePerMatch: false, cooldownSeconds: 0, enabled: true },
    preset: "zzz-onedragon" },
];
// i18n-ignore-end

/** 测试和演示可以在页面加载前放 `window.__kotoneAutomationSeed`（`[{name, source, createdBy?, description?}]`）：当作她或
 * 外部 Agent 存的（默认 companion），加在示例数据后面。 */
type PreviewSeed = { name: string; source: string; createdBy?: Automation["createdBy"]; description?: string };
const seeds = typeof window === "undefined" ? undefined : (window as unknown as { __kotoneAutomationSeed?: PreviewSeed[] }).__kotoneAutomationSeed;
previewAutomations = [...previewAutomations, ...(Array.isArray(seeds) ? seeds : []).map((seed, index): PreviewAutomation => ({
  id: `preview-seed-${index + 1}`, name: seed.name, description: seed.description ?? "", createdBy: seed.createdBy ?? "companion",
  createdAtMs: t0, updatedAtMs: t0, params: [], action: { kind: "program", source: seed.source }, ...previewUses(seed.source),
}))];

/** 预览里玩家点过“允许”的：自动化 id → 允许时的那份程序和允许的工具（真实应用记在定义上；程序一改就作废，ADR 046 D3）。 */
const previewAllowed = new Map<string, { source: string; tools: string[] }>();
function describePreview(a: PreviewAutomation): Automation {
  const source = a.action.source ?? "";
  const allowed = previewAllowed.get(a.id);
  const { tools, needsConfirm } = previewToolStates(source, allowed && allowed.source === source.trim() ? allowed.tools : []);
  return { ...structuredClone(a), tools, needsConfirm };
}
/** 照这份程序记下要允许的工具（和以前允许过的合在一起）。 */
function previewAllow(a: PreviewAutomation) {
  const source = (a.action.source ?? "").trim();
  const before = previewAllowed.get(a.id);
  const kept = before && before.source === source ? before.tools : [];
  const now = previewToolStates(source, kept).tools.filter((t) => t.state === "confirm").map((t) => t.name);
  previewAllowed.set(a.id, { source, tools: [...kept, ...now] });
}

const previewListeners = new Set<() => void>();
const notify = () => previewListeners.forEach((listener) => listener());
const PREVIEW_ONLY = m.auto_preview_only();
/** 列表里不带源码（和后端 `automation_saved_list` 一样），要源码用 getAutomation。 */
const listed = (a: PreviewAutomation): Automation => ({ ...describePreview(a), action: { kind: "program" } });

// ---------- 调用 ----------

export async function listRuns(): Promise<RunsSnapshot> {
  if (isTauri) return invoke("automation_runs");
  const runs = structuredClone(previewRuns);
  return {
    // 名额只数绑了游戏窗口、正在跑的（ADR 045 D5）
    running: runs.filter((r) => r.status === "running" && r.action.target).length,
    pending: runs.filter((r) => r.status === "pending").length,
    maxRunning: 3,
    runs,
  };
}

/** 取消 / 停止一条执行；不给 id = 停下（ADR 045 D6）：运行中的全停，等待中的只取消绑了游戏窗口的，等待中的提醒保留。返回涉及条数。 */
export async function stopRuns(id?: string): Promise<number> {
  if (isTauri) return invoke("automation_stop", { id: id ?? null });
  let count = 0;
  for (const run of previewRuns) {
    const open = run.status === "pending" || run.status === "running";
    const hit = id ? run.id === id : run.status === "running" || !!run.action.target;
    if (open && hit) {
      run.status = "cancelled";
      run.message = "用户停止"; // i18n-ignore：预览里模拟后端写的原因
      run.finishedAtMs = Date.now();
      count++;
    }
  }
  notify();
  return count;
}

/** 已保存的自动化、上限和新建用的模板。 */
export async function listSaved(): Promise<SavedList> {
  if (isTauri) return invoke("automation_saved_list");
  return { automations: previewAutomations.map(listed), max: 200, templates: structuredClone(previewTemplates) };
}

export async function listAutomations(): Promise<Automation[]> {
  return (await listSaved()).automations;
}

export async function getAutomation(id: string): Promise<Automation> {
  if (isTauri) return invoke("automation_get", { id });
  const found = previewAutomations.find((a) => a.id === id);
  if (!found) throw new Error(`找不到自动化 ${id}`); // i18n-ignore：预览里模拟后端的错误
  return describePreview(found);
}

/** 保存。`confirm` = 玩家点了“允许并保存”（ADR 046 D9）：存好以后照刚存的这份程序允许要确认的工具。
 * 改了程序的，存的时候就要再允许一次（存的时候以前的允许已经作废了）。 */
export async function saveAutomation(draft: AutomationDraft, confirm = false): Promise<Automation> {
  if (isTauri) return invoke("automation_save", { draft, confirm });
  // 草稿可能带着界面状态的代理：存成普通 JSON，之后 structuredClone 才不会出错
  draft = JSON.parse(JSON.stringify(draft)) as AutomationDraft;
  // i18n-ignore-start：预览里模拟后端的错误
  if (!draft.name.trim()) throw new Error("自动化名字不能为空，最多 40 字");
  if (!draft.action.source?.trim()) throw new Error("自动化内容为空或超过 8000 字");
  // i18n-ignore-end
  const now = Date.now();
  const existing = draft.id ? previewAutomations.find((a) => a.id === draft.id) : undefined;
  const source = draft.action.source.trim();
  const { error } = previewToolNames(source);
  if (error) throw new Error(error);
  const saved: PreviewAutomation = {
    id: existing?.id ?? `preview-auto-${now}`, name: draft.name.trim(), description: draft.description.trim(),
    params: draft.params.map((p) => ({ ...p, label: p.label.trim() || p.name })), action: { kind: "program", source },
    createdBy: existing?.createdBy ?? "user", createdAtMs: existing?.createdAtMs ?? now, updatedAtMs: now,
    schedule: draft.schedule ?? null, nextRunAtMs: draft.schedule?.enabled ? now + 3_600_000 : null,
    on: draft.on ?? null,
    ...previewUses(source),
  };
  previewAutomations = existing ? previewAutomations.map((a) => (a.id === saved.id ? saved : a)) : [...previewAutomations, saved];
  if (confirm) previewAllow(saved);
  notify();
  return describePreview(saved);
}

/** 列表上点“允许”（ADR 046 D3）：`source` 是界面上看到的那份程序，和现在存的不一样（这期间被她、外部 Agent 改了）就不认。
 * 返回允许后的定义（带源码）。 */
export async function confirmAutomation(id: string, source: string): Promise<Automation> {
  if (isTauri) return invoke("automation_confirm", { id, source });
  const found = previewAutomations.find((a) => a.id === id);
  // i18n-ignore-start：预览里模拟后端的错误
  if (!found) throw new Error(`找不到自动化 ${id}`);
  if ((found.action.source ?? "").trim() !== source.trim()) throw new Error("程序刚被改过，请重新看一遍");
  // i18n-ignore-end
  previewAllow(found);
  notify();
  return describePreview(found);
}

/** 编辑器里程序下面那一行“会调用：…”：照编辑器里现在这份程序（还没存）算；`id` 是正在改的那个（新建的不传），
 * 程序没改过时它允许过的照样算数。 */
export async function programTools(source: string, id?: string | null): Promise<ToolsPreview> {
  if (isTauri) return invoke("automation_tools", { source, id: id ?? null });
  const allowed = id ? previewAllowed.get(id) : undefined;
  return previewToolStates(source, allowed && allowed.source === source.trim() ? allowed.tools : []);
}

export async function deleteAutomation(id: string): Promise<boolean> {
  if (isTauri) return invoke("automation_delete", { id });
  const before = previewAutomations.length;
  previewAutomations = previewAutomations.filter((a) => a.id !== id);
  previewAllowed.delete(id);
  notify();
  return previewAutomations.length !== before;
}

/** 从设置页运行（▶）：用到游戏窗口的倒数 switchSeconds 秒（切到游戏）后以那时的前台窗口为目标；
 * 不用的直接排上（声明了数字参数 delay_seconds 的晚这么多秒开始）。 */
export async function runAutomation(id: string, args: Record<string, unknown>, switchSeconds: number): Promise<RunSavedResult> {
  if (isTauri) return invoke("automation_run_saved", { id, args, switchSeconds });
  // 浏览器预览：只在列表里加一条示例执行，不会真的按键或出声
  const automation = previewAutomations.find((a) => a.id === id);
  // i18n-ignore-start：预览里模拟后端的错误和示例窗口
  if (!automation) throw new Error(`找不到自动化 ${id}`);
  // 等你确认的、调不了的不运行（ADR 046 D3），和后端一样说清是哪个工具
  const problems = describePreview(automation).tools.filter((t) => t.state !== "ready")
    .map((t) => t.state === "confirm" ? `要先在「自动化」页点“允许”，才能调用 ${t.name}（${t.reason}）` : t.reason.includes(t.name) ? t.reason : `${t.name}：${t.reason}`);
  if (problems.length) throw new Error(problems.join("；"));
  const now = Date.now();
  const missing = automation.params.find((p) => (p.default === undefined || p.default === null) && (args[p.name] === undefined || args[p.name] === ""));
  if (missing) throw new Error(`缺少参数 ${missing.label || missing.name}`);
  const values = Object.fromEntries(automation.params.flatMap((p) => {
    const value = args[p.name] ?? p.default;
    return value === undefined || value === null ? [] : [[p.name, value]];
  }));
  const action: RunAction = { kind: "program", name: automation.name, source: automation.action.source, automationId: id, args: values, target: null };
  const runId = `preview-run-${now}`;
  if (automation.usesWindow) {
    previewRuns = [...previewRuns, { id: runId, sessionId: "ui", status: "running", createdAtMs: now, dueAtMs: now, startedAtMs: now,
      action: { ...action, target: { handle: 1, pid: 1, label: "示例游戏 (game.exe)" } } }];
    notify();
    return { status: "countdown", seconds: switchSeconds };
  }
  // i18n-ignore-end
  const delay = automation.params.some((p) => p.name === "delay_seconds" && p.type === "number") ? Number(values.delay_seconds ?? 0) * 1000 : 0;
  const dueAtMs = now + delay;
  // 预览不出声：马上开始的直接记成跑完
  previewRuns = [...previewRuns, delay > 0
    ? { id: runId, sessionId: "ui", status: "pending", createdAtMs: now, dueAtMs, action }
    : { id: runId, sessionId: "ui", status: "finished", createdAtMs: now, dueAtMs, startedAtMs: now, finishedAtMs: now, action, message: PREVIEW_ONLY }];
  notify();
  return { status: delay > 0 ? "scheduled" : "started", id: runId, target: null, dueAtUnixMs: dueAtMs };
}

export async function subscribeAutomations(callback: () => void): Promise<UnlistenFn> {
  if (isTauri) return listen("kotone://automations-updated", callback);
  previewListeners.add(callback);
  return () => { previewListeners.delete(callback); };
}
