// 语音唤醒（ADR 017）：状态、模型下载与事件。设置本身在 config.json `assistant.wake`，用普通设置补丁改。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";

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
  { value: "qinyin-qinyin", label: "琴音琴音" },
  { value: "nihao-qinyin", label: "你好琴音" },
] as const;

export const wakeSensitivities = [
  { value: "low", label: "低（不容易误唤醒）" },
  { value: "normal", label: "中" },
  { value: "high", label: "高（更容易唤醒）" },
] as const;

export const wakeAcks = [
  { value: "voice", label: "说“嗯？/我在”" },
  { value: "tone", label: "提示音" },
  { value: "none", label: "不提示" },
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
    case "off": return "未开启";
    case "waitingRuntime": return "本地语音识别启动后开始听";
    case "modelMissing": return "还没有下载唤醒模型";
    case "listening": return "正在监听唤醒词";
    case "error": return `出错了：${status.message ?? "未知原因"}`;
  }
}
