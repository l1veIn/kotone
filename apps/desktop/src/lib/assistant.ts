import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import * as browserMock from "./ipc/browserMock";
import { m } from "./paraglide/messages.js";

export type DeliveryKind = "voice" | "danmaku";
export interface SpeechVoice { id: string; name: string }
export interface SpeechModelStatus {
  modelId: string;
  installed: boolean;
  downloading: boolean;
  downloadedBytes: number;
  totalBytes: number;
  phase: "idle" | "downloading" | "verifying" | "extracting" | "installed" | "error" | "cancelled";
  error: string | null;
  modelDir: string;
}

export async function getSpeechModelStatus(): Promise<SpeechModelStatus> {
  return isTauri ? invoke("get_speech_model_status") : {
    modelId: "kokoro-int8-multi-lang-v1_1", installed: false, downloading: false,
    // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的数据），不是界面文字
    downloadedBytes: 0, totalBytes: 147031220, phase: "idle", error: null, modelDir: "界面预览",
    // i18n-ignore-end
  };
}
export async function downloadSpeechModel(): Promise<SpeechModelStatus> {
  if (!isTauri) throw new Error(m.shell_assistant_speech_download_desktop_only());
  return invoke("download_speech_model");
}
export async function cancelSpeechModelDownload(): Promise<void> {
  if (isTauri) await invoke("cancel_speech_model_download");
}
export async function subscribeSpeechModel(callback: (status: SpeechModelStatus) => void): Promise<UnlistenFn> {
  return isTauri ? listen<SpeechModelStatus>("kotone://speech-model", (event) => callback(event.payload)) : () => {};
}

export async function listSpeechVoices(): Promise<SpeechVoice[]> {
  return isTauri ? invoke("list_speech_voices") : browserMock.speechVoices();
}

/** Key 从哪来（ADR 048；不含 Key 本身）。 */
export type SpeechKeySource =
  | { kind: "own" }
  | { kind: "connection"; id: string; name: string }
  | { kind: "speechRecognition" }
  | { kind: "notNeeded" }
  | { kind: "none" };
/** 一家在线声音的状态（`speech_providers`）。显示名和计费说明在界面文案里按 `modelId` 取。 */
export interface SpeechProvider {
  modelId: string;
  /** 声音编号的前缀（`minimax/…`）。 */
  provider: string;
  apiKeyUrl: string | null;
  defaultModel: string;
  defaultVoice: string;
  /** 能不能调语速。 */
  rate: boolean;
  /** Key 分国内 / 国际站（保存时自动判断）。 */
  regions: boolean;
  /** 只要一个 Key。 */
  keyOnly: boolean;
  active: boolean;
  ready: boolean;
  reason: string | null;
  keySource: SpeechKeySource;
  region: "cn" | "intl" | null;
}
/** 她现在用的声音（`speech_current`）：“自动”时是哪一家。 */
export interface SpeechCurrent {
  modelId: string;
  auto: boolean;
  online: boolean;
  ready: boolean;
  reason: string | null;
  keySource: SpeechKeySource;
  region: "cn" | "intl" | null;
  rate: boolean;
}
/** 保存 Key / 先试一次的结果。 */
export interface SpeechCheck { ok: boolean; saved: boolean; message: string; region: "cn" | "intl" | null }

export async function listSpeechProviders(): Promise<SpeechProvider[]> {
  return isTauri ? invoke("speech_providers") : browserMock.speechProviders() as Promise<SpeechProvider[]>;
}
export async function getSpeechCurrent(): Promise<SpeechCurrent> {
  return isTauri ? invoke("speech_current") : browserMock.speechCurrent() as Promise<SpeechCurrent>;
}
/** 保存这一家的 Key：先试一次（MiniMax、百炼顺便判断国内 / 国际站），Key 被拒就不存。 */
export async function saveSpeechKey(modelId: string, key: string): Promise<SpeechCheck> {
  return isTauri ? invoke("speech_key_save", { modelId, key }) : browserMock.speechKeySave(modelId, key) as Promise<SpeechCheck>;
}
/** 清掉声音设置里为这一家填的 Key（同一家 API 连接的 Key 不动）。 */
export async function clearSpeechKey(modelId: string): Promise<void> {
  return isTauri ? invoke("speech_key_clear", { modelId }) : browserMock.speechKeyClear(modelId);
}
/** 试听一个声音（`提供方/声音`）：念一句固定的话。 */
export async function tryVoice(voiceId: string): Promise<void> {
  if (!isTauri) throw new Error(m.shell_assistant_preview_output_desktop_only());
  await invoke("speech_try_voice", { voiceId });
}
export async function previewReminder(kind: DeliveryKind): Promise<void> {
  if (!isTauri) throw new Error(m.shell_assistant_preview_output_desktop_only());
  await invoke("preview_reminder", { kind });
}
/** 呼叫琴音（和热键一样）：按一下就是通话（ADR 033），一直在听；通话中再按一下挂断。 */
export async function callKotone(): Promise<void> {
  if (!isTauri) throw new Error(m.shell_assistant_call_desktop_only());
  await invoke("assistant_call");
}
/** 现在在不在和她通话（ADR 033）。 */
export async function callActive(): Promise<boolean> {
  return isTauri ? invoke<boolean>("call_active") : false;
}
/** 接通 / 挂断时通知。 */
export async function subscribeCall(callback: (active: boolean) => void): Promise<UnlistenFn> {
  return isTauri ? listen<{ active: boolean }>("kotone://call", ({ payload }) => callback(payload.active)) : () => {};
}
export async function stopSpeech(): Promise<void> {
  if (isTauri) await invoke("stop_speech");
}
