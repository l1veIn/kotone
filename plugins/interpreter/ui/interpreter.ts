// 同传插件（ADR 021、038）：页面上设好听哪个程序、译成什么、怎么显示，点“开始”就等于玩家在聊天里对她说了那句话，
// 她交给后台去听、去翻（后端 plugins/interpreter/src/lib.rs）。工具 `interpreter.*` 只给页面用：
// status 读设置和她的耳朵现在在听哪些程序，start / stop 替玩家对她说一句。
// 设置在 `plugins.interpreter.settings`（app、target、display，和后端的 Prefs 一致）；插件关着时工具只回 `{status: "disabled"}`。
import type { Settings } from "$lib/ipc/contracts";
import { updateSettings } from "$lib/ipc";
import { pluginCall } from "$lib/plugins";
import { settingsStore } from "$lib/stores/ui";

export const PLUGIN_ID = "interpreter";
export const PAGE_HOME = "ui://kotone/interpreter/home";

export type InterpreterDisplay = "danmaku" | "bubble";

export interface InterpreterStatus {
  /** 她的耳朵现在在听的程序（进程名）。 */
  listening: string[];
  /** 听哪个程序；空 = 现在的游戏。 */
  app: string;
  target: string;
  display: InterpreterDisplay;
  /** 能译成的语言（第一个是默认）。 */
  targets: string[];
}

/** 插件自己的设置（`plugins.interpreter.settings`）。 */
export interface InterpreterPrefs {
  app: string;
  target: string;
  display: InterpreterDisplay;
}

/** 设置里现在的偏好；语言不在 `targets` 里时按第一个（和后端一样）。 */
export function interpreterPrefs(settings: Settings | null | undefined, targets: string[]): InterpreterPrefs {
  const raw = (settings?.plugins?.[PLUGIN_ID]?.settings ?? {}) as Record<string, unknown>;
  const target = typeof raw.target === "string" ? raw.target.trim() : "";
  return {
    app: typeof raw.app === "string" ? raw.app : "",
    target: targets.includes(target) ? target : (targets[0] ?? ""),
    display: raw.display === "bubble" ? "bubble" : "danmaku",
  };
}

/** 存一部分偏好。 */
export async function saveInterpreter(patch: Partial<InterpreterPrefs>): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [PLUGIN_ID]: { settings: patch } } }));
}

/** 状态；插件关着时 null。 */
export async function interpreterStatus(): Promise<InterpreterStatus | null> {
  const result = await pluginCall(PLUGIN_ID, "interpreter.status");
  const content = result.content as (InterpreterStatus & { status?: string }) | null;
  return content && content.status !== "disabled" ? content : null;
}

/** 替玩家对她说一句（开始 / 停止同传）；回替玩家说了什么。插件关着时报错。 */
async function say(tool: "interpreter.start" | "interpreter.stop"): Promise<string> {
  const result = await pluginCall(PLUGIN_ID, tool);
  const content = result.content as { said?: string; status?: string; text?: string } | null;
  if (content?.status === "disabled") throw new Error(content.text ?? "disabled");
  return content?.said ?? "";
}

export const interpreterStart = () => say("interpreter.start");
export const interpreterStop = () => say("interpreter.stop");
