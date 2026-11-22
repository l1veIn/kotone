// 语音唤醒（ADR 017）：状态、模型下载与事件。设置本身在 config.json `assistant.wake`，用普通设置补丁改。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";
import { m } from "./paraglide/messages.js";

export type WakeStateName = "off" | "waitingRuntime" | "modelMissing" | "listening" | "error";

export interface WakeStatus {
  state: WakeStateName;
  /** state = error 时的原因 */
  message?: string;
  modelReady: boolean;
  downloading: boolean;
  /** 本次启动以来唤醒了几次 */
  detections: number;
}

export interface WakeModelProgress {
  downloadedBytes: number;
  totalBytes: number | null;
}

export const wakePhrases = [
  // i18n-ignore-start：唤醒词是玩家要说的话，不是界面文字
  { value: "qinyin-qinyin", label: "琴音琴音" },
  { value: "nihao-qinyin", label: "你好琴音" },
  // i18n-ignore-end
] as const;

export const wakeSensitivities = [
  { value: "low", label: m.misc_wake_sensitivity_low() },
  { value: "normal", label: m.misc_wake_sensitivity_normal() },
  { value: "high", label: m.misc_wake_sensitivity_high() },
] as const;

export const wakeAcks = [
  { value: "voice", label: m.misc_wake_ack_voice() },
  { value: "tone", label: m.misc_wake_ack_tone() },
  { value: "none", label: m.misc_wake_ack_none() },
] as const;

// ---------- 浏览器预览 ----------
const preview: { status: WakeStatus; listeners: Set<(s: WakeStatus) => void> } = {
  status: { state: "off", modelReady: false, downloading: false, detections: 0 },
  listeners: new Set(),
};

function previewStatus(enabled: boolean): WakeStatus {
  const s = preview.status;
  const state: WakeStateName = !enabled ? "off" : !s.modelReady ? "modelMissing" : "waitingRuntime";
  return { ...s, state };
}

export async function wakeStatus(enabled = false): Promise<WakeStatus> {
  return isTauri ? invoke<WakeStatus>("wake_status") : previewStatus(enabled);
}

export async function wakeDownloadModel(enabled = false): Promise<WakeStatus> {
  if (isTauri) return invoke<WakeStatus>("wake_download_model");
  preview.status = { ...preview.status, downloading: true };
  await new Promise((resolve) => setTimeout(resolve, 300));
  preview.status = { ...preview.status, downloading: false, modelReady: true };
  const next = previewStatus(enabled);
  preview.listeners.forEach((listener) => listener(next));
  return next;
}

export async function subscribeWake(callback: (status: WakeStatus) => void): Promise<UnlistenFn> {
  if (isTauri) return listen<WakeStatus>("kotone://wake-status", ({ payload }) => callback(payload));
  preview.listeners.add(callback);
  return () => { preview.listeners.delete(callback); };
}

export async function subscribeWakeModel(callback: (progress: WakeModelProgress) => void): Promise<UnlistenFn> {
  return isTauri ? listen<WakeModelProgress>("kotone://wake-model", ({ payload }) => callback(payload)) : () => {};
}

export function wakeStateText(status: WakeStatus): string {
  switch (status.state) {
    case "off": return m.misc_wake_state_off();
    case "waitingRuntime": return m.misc_wake_state_waiting_runtime();
    case "modelMissing": return m.misc_wake_state_model_missing();
    case "listening": return m.misc_wake_state_listening();
    case "error": return m.misc_wake_state_error({ message: status.message ?? m.misc_wake_state_error_unknown() });
  }
}
