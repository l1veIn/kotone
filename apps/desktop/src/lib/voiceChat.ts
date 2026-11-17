// 游戏语音（ADR 020）：琴音用虚拟麦克风（VB-CABLE）替你在游戏语音里说话。
// 状态、自动安装、改 / 改回 Windows 默认麦克风、测试、说一句；开关等设置在 config.json `voiceChat`，用普通设置补丁改。
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";

export interface CaptureEndpoint {
  id: string;
  name: string;
}

export interface VoiceChatStatus {
  driverInstalled: boolean;
  installing: boolean;
  devices: { cableInput: string | null; cableOutput: string | null };
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

// ---------- 浏览器预览 ----------
const preview = {
  installed: false,
  defaultIsCable: false,
  restorable: false,
};

function previewStatus(enabled: boolean): VoiceChatStatus {
  return {
    driverInstalled: preview.installed,
    installing: false,
    devices: preview.installed
      ? { cableInput: "CABLE Input (VB-Audio Virtual Cable)", cableOutput: "CABLE Output (VB-Audio Virtual Cable)" }
      : { cableInput: null, cableOutput: null },
    enabled,
    includeMic: true,
    micDeviceId: "default",
    micGain: 100,
    running: enabled && preview.installed,
    error: null,
    windowsDefaultMic: { id: "mic", name: preview.defaultIsCable ? "CABLE Output (VB-Audio Virtual Cable)" : "麦克风 (USB Audio)" },
    windowsCommunicationsMic: { id: "mic", name: preview.defaultIsCable ? "CABLE Output (VB-Audio Virtual Cable)" : "麦克风 (USB Audio)" },
    windowsDefaultIsCable: preview.defaultIsCable,
    canRestoreDefaultMic: preview.restorable,
    ttsCanSpeak: true,
    attribution: "VB-CABLE 是 VB-Audio 的捐赠软件（https://vb-cable.com），安装即表示同意它的许可；欢迎到官网捐助作者。",
  };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function voiceChatStatus(enabled = false): Promise<VoiceChatStatus> {
  return isTauri ? invoke<VoiceChatStatus>("voice_chat_status") : previewStatus(enabled);
}

export interface InstallResult {
  installed: boolean;
  needsReboot?: boolean;
  alreadyInstalled?: boolean;
  note?: string;
}

export async function voiceChatInstall(): Promise<InstallResult> {
  if (isTauri) return invoke<InstallResult>("voice_chat_install");
  await wait(400);
  preview.installed = true;
  return { installed: true, needsReboot: false };
}

export async function voiceChatDefaultMic(useCable: boolean): Promise<{ defaultMic: string | null }> {
  if (isTauri) return invoke("voice_chat_default_mic", { useCable });
  preview.defaultIsCable = useCable;
  preview.restorable = useCable;
  return { defaultMic: useCable ? "CABLE Output (VB-Audio Virtual Cable)" : "麦克风 (USB Audio)" };
}

export async function voiceChatTest(): Promise<{ ok: boolean; peak: number; note: string }> {
  if (isTauri) return invoke("voice_chat_test");
  await wait(300);
  return { ok: true, peak: 0.3, note: "通了：游戏或语音软件的麦克风选 CABLE Output，队友就能听到" };
}

export async function voiceChatSay(text: string): Promise<{ sent: boolean; durationMs: number }> {
  if (isTauri) return invoke("voice_chat_say", { text });
  return { sent: true, durationMs: 1200 };
}
