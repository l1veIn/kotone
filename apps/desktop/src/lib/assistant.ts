import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri, type AssistantInteractionMode } from "./ipc";
import * as browserMock from "./ipc/browserMock";
import { m } from "./paraglide/messages.js";

export interface AssistantVoiceSession { sessionId: string; interactionMode: AssistantInteractionMode }
export async function beginAssistantVoice(): Promise<AssistantVoiceSession> {
  if (!isTauri) throw new Error(m.shell_assistant_voice_desktop_only());
  return invoke("assistant_voice_begin");
}
export async function endAssistantVoice(sessionId: string): Promise<void> {
  if (isTauri) await invoke("assistant_voice_end", { sessionId });
}
export async function confirmAssistantVoice(sessionId: string): Promise<void> {
  if (isTauri) await invoke("assistant_voice_confirm", { sessionId });
}
export async function cancelAssistantVoice(sessionId: string): Promise<void> {
  if (isTauri) await invoke("assistant_voice_cancel", { sessionId });
}

export type DeliveryKind = "voice" | "danmaku";
export interface Reminder {
  id: string;
  sessionId: string;
  text: string;
  delivery: DeliveryKind[];
  createdAtMs: number;
  dueAtMs: number;
  status: "pending" | "delivering" | "delivered" | "failed" | "cancelled" | "missed";
  error?: string | null;
}
export interface ReminderIntent { delaySeconds: number; text: string; delivery: DeliveryKind[] }
export interface AssistantPlan { message: string; reminders: ReminderIntent[] }
export interface AssistantEvent { sessionId: string; text: string; message: string; reminders: Reminder[] }
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

// Browser preview uses labelled, local fixtures; it never pretends to run AI or schedule system output.
const previewTime = Date.now();
let previewReminders: Reminder[] = [{
  id: "preview-reminder", sessionId: "preview", text: "界面示例：打野有闪了",
  delivery: ["voice", "danmaku"], createdAtMs: previewTime, dueAtMs: previewTime + 300_000,
  status: "pending", error: null,
}];
const previewListeners = new Set<() => void>();

export function addPreviewReminder(): void {
  if (isTauri) throw new Error(m.shell_assistant_preview_reminder_desktop());
  const createdAtMs = Date.now();
  previewReminders = [{
    id: `preview-${crypto.randomUUID()}`, sessionId: "preview", text: "界面示例：该打小龙了",
    delivery: ["voice", "danmaku"], createdAtMs, dueAtMs: createdAtMs + 300_000, status: "pending",
  }, ...previewReminders];
  previewListeners.forEach((notify) => notify());
}

export async function listReminders(): Promise<Reminder[]> {
  return isTauri ? invoke("list_reminders") : structuredClone(previewReminders);
}
export async function cancelReminder(id: string): Promise<boolean> {
  if (isTauri) return invoke("cancel_reminder", { id });
  const reminder = previewReminders.find((item) => item.id === id && item.status === "pending");
  if (!reminder) return false;
  reminder.status = "cancelled";
  previewListeners.forEach((notify) => notify());
  return true;
}
// i18n-ignore-end
export async function submitAssistantText(text: string, legacyReminder?: boolean): Promise<AssistantPlan> {
  if (!isTauri) throw new Error(m.shell_assistant_reminder_desktop_only());
  return invoke("assistant_submit_text", { text, legacyReminder });
}
export async function cancelAssistantRequest(): Promise<void> {
  if (isTauri) await invoke("cancel_assistant_request");
}
export async function listSpeechVoices(legacyReminder?: boolean): Promise<SpeechVoice[]> {
  return isTauri ? invoke("list_speech_voices", { legacyReminder }) : browserMock.speechVoices();
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
export async function previewReminder(kind: DeliveryKind, legacyReminder?: boolean): Promise<void> {
  if (!isTauri) throw new Error(m.shell_assistant_preview_output_desktop_only());
  await invoke("preview_reminder", { kind, legacyReminder });
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
export async function subscribeReminders(callback: () => void): Promise<UnlistenFn> {
  if (isTauri) return listen("kotone://reminders-updated", callback);
  previewListeners.add(callback);
  return () => { previewListeners.delete(callback); };
}
export async function subscribeAssistant(callback: (event: AssistantEvent) => void): Promise<UnlistenFn> {
  return isTauri ? listen<AssistantEvent>("kotone://assistant", ({ payload }) => callback(payload)) : () => {};
}
