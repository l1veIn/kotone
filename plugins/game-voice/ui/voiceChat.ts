// 游戏语音插件（ADR 020、038）：她用虚拟麦克风（VB-CABLE）替你在游戏语音里说话。
// 页面经插件工具（`voice_chat.*`，和她、外部 Agent 用的是同一个实现）读状态、自动安装、改 / 改回 Windows 默认麦克风、
// 测试、说一句。开关是插件开关（`plugins.game-voice.enabled`）；自己的麦克风、音量在 `plugins.game-voice.settings`。
// 插件关着时工具只回 `{status: "disabled"}`，页面那时只显示开关。
import type { Settings } from "$lib/ipc/contracts";
import { pluginCall } from "$lib/plugins";

export const PLUGIN_ID = "game-voice";
export const PAGE_HOME = "ui://kotone/game-voice/home";
/** 推送“她对队友说了”的卡片（ADR 043 D11，后端 plugins/game-voice/src/pushes.rs）。 */
export const PAGE_SAID = "ui://kotone/game-voice/said";

/** “她对队友说了”卡片里的数据：上一句说完后 2 分钟内的几句，一句一行。 */
export interface SaidCardData {
  lines: { text: string; atMs: number }[];
}

export interface CaptureEndpoint {
  id: string;
  name: string;
}

export interface VoiceChatStatus {
  driverInstalled: boolean;
  installing: boolean;
  devices: { cableInput: string | null; cableOutput: string | null };
  /** 插件开着（转发开着） */
  enabled: boolean;
  includeMic: boolean;
  micDeviceId: string;
  micGain: number;
  /** 转发在运行 */
  running: boolean;
  error: string | null;
  windowsDefaultMic: CaptureEndpoint | null;
  windowsCommunicationsMic: CaptureEndpoint | null;
  windowsDefaultIsCable: boolean;
  canRestoreDefaultMic: boolean;
  /** 当前语音引擎能不能送进游戏（系统语音不行） */
  ttsCanSpeak: boolean;
  attribution: string;
}

/** 插件自己的设置（`plugins.game-voice.settings`，和后端 plugins/game-voice/src/config.rs 一致）。 */
export interface VoiceChatPrefs {
  includeMic: boolean;
  micDeviceId: string;
  micGain: number;
}

/** 设置里现在的偏好（没写的按默认：混入自己的麦克风、系统默认麦克风、100%）。 */
export function voiceChatPrefs(settings: Settings | null | undefined): VoiceChatPrefs {
  const raw = (settings?.plugins?.[PLUGIN_ID]?.settings ?? {}) as Record<string, unknown>;
  const gain = typeof raw.micGain === "number" && Number.isFinite(raw.micGain) ? Math.round(raw.micGain) : 100;
  return {
    includeMic: raw.includeMic !== false,
    micDeviceId: typeof raw.micDeviceId === "string" && raw.micDeviceId.trim() ? raw.micDeviceId : "default",
    micGain: Math.min(Math.max(gain, 0), 400),
  };
}

/** 调本插件的工具：插件关着时报错（页面那时不该调）。 */
async function call<T>(tool: string, args: Record<string, unknown> = {}): Promise<T> {
  const result = await pluginCall(PLUGIN_ID, tool, args);
  const content = result.content as (T & { status?: string; text?: string }) | null;
  if (content && typeof content === "object" && content.status === "disabled") throw new Error(content.text ?? "disabled");
  return content as T;
}

/** 状态；插件关着时 null。 */
export async function voiceChatStatus(): Promise<VoiceChatStatus | null> {
  const result = await pluginCall(PLUGIN_ID, "voice_chat.status");
  const content = result.content as (VoiceChatStatus & { status?: string }) | null;
  return content && content.status !== "disabled" ? content : null;
}

export interface InstallResult {
  installed: boolean;
  needsReboot?: boolean;
  alreadyInstalled?: boolean;
  note?: string;
}

export const voiceChatInstall = () => call<InstallResult>("voice_chat.install");

export const voiceChatDefaultMic = (useCable: boolean) =>
  call<{ defaultMic: string | null }>("voice_chat.default_mic", { use: useCable ? "cable" : "restore" });

export const voiceChatTest = () => call<{ ok: boolean; peak: number; note: string }>("voice_chat.test");

export const voiceChatSay = (text: string) => call<{ sent: boolean; durationMs: number }>("voice_chat.say", { text });
