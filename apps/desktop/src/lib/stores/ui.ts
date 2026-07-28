/*
 * 设置窗口共享 UI 状态（方向 B 重构）：
 * - settingsStore：Settings 唯一前端副本，壳加载一次，各设置页读写；
 *   所有变更走 updateSettings（后端深合并 + 落盘）后整体替换。
 * - toasts：右上角 toast 堆叠（Toasts.svelte 呈现）。四种类型：
 *   success 青 / info 白 / warning 黄 / error 品红；success·info·warning
 *   4s 自动消隐，error 8s；均可点击关闭。
 *   toast(ok, text) 为旧布尔签名兼容封装（true→success，false→error）。
 */

import { writable } from "svelte/store";
import { logFrontendError } from "../ipc";
import type { Settings } from "../ipc";

export const settingsStore = writable<Settings | null>(null);

export type ToastKind = "success" | "info" | "warning" | "error";

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

export const toasts = writable<ToastItem[]>([]);

let seq = 0;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

/** 推一条 toast；error 停留更久（8s），其余 4s。返回 id 供手动关闭。 */
export function pushToast(kind: ToastKind, text: string): number {
  const id = ++seq;
  toasts.update((list) => [...list, { id, kind, text }]);
  if (kind === "error") {
    void logFrontendError("toast", text).catch(() => {});
  }
  timers.set(
    id,
    setTimeout(() => dismissToast(id), kind === "error" ? 8000 : 4000),
  );
  return id;
}

export function dismissToast(id: number): void {
  const t = timers.get(id);
  if (t) {
    clearTimeout(t);
    timers.delete(id);
  }
  toasts.update((list) => list.filter((x) => x.id !== id));
}

/** 旧布尔签名兼容：true → success（青），false → error（品红） */
export function toast(ok: boolean, text: string): void {
  pushToast(ok ? "success" : "error", text);
}

export const toastInfo = (text: string): number => pushToast("info", text);
export const toastWarn = (text: string): number => pushToast("warning", text);

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
