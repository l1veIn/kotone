/*
 * 设置窗口共享 UI 状态（方向 B 重构）：
 * - settingsStore：Settings 唯一前端副本，壳加载一次，各设置页读写；
 *   所有变更走 updateSettings（后端深合并 + 落盘）后整体替换。
 * - feedback / toast：底部反馈条（ok 青色 / 错误品红，3s 自动消隐）。
 */

import { writable } from "svelte/store";
import type { Settings } from "../ipc";

export const settingsStore = writable<Settings | null>(null);

export interface Feedback {
  ok: boolean;
  text: string;
}

export const feedback = writable<Feedback | null>(null);

let toastTimer: ReturnType<typeof setTimeout> | null = null;

export function toast(ok: boolean, text: string): void {
  feedback.set({ ok, text });
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => feedback.set(null), 3000);
}

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
