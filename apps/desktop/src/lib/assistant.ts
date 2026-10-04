import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri, type AssistantInteractionMode } from "./ipc";

export interface AssistantVoiceSession { sessionId: string; interactionMode: AssistantInteractionMode }
export async function beginAssistantVoice(): Promise<AssistantVoiceSession> {
  if (!isTauri) throw new Error("浏览器仅预览界面；请在桌面应用中使用语音输入。");
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
    downloadedBytes: 0, totalBytes: 147031220, phase: "idle", error: null, modelDir: "界面预览",
  };
}
export async function downloadSpeechModel(): Promise<SpeechModelStatus> {
  if (!isTauri) throw new Error("请在桌面应用中下载语音模型。");
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
  if (isTauri) throw new Error("桌面应用不使用预览提醒");
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
export async function submitAssistantText(text: string): Promise<AssistantPlan> {
  if (!isTauri) throw new Error("浏览器仅预览界面；请在桌面应用中连接 AI 后创建提醒。");
  return invoke("assistant_submit_text", { text });
}
export async function cancelAssistantRequest(): Promise<void> {
  if (isTauri) await invoke("cancel_assistant_request");
}
export async function listSpeechVoices(): Promise<SpeechVoice[]> {
  return isTauri ? invoke("list_speech_voices") : [
    {id:"kokoro:zf_001",name:"中文女声 001"}, {id:"kokoro:zf_003",name:"中文女声 003"}, {id:"kokoro:zf_017",name:"中文女声 017"},
  ];
}
export async function previewReminder(kind: DeliveryKind): Promise<void> {
  if (!isTauri) throw new Error("浏览器预览不播放语音或显示系统弹幕；请在桌面应用中测试。");
  await invoke("preview_reminder", { kind });
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
