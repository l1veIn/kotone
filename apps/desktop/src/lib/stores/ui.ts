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
import { logFrontendError, updateSettings } from "../ipc";
import type { Settings } from "../ipc";

export const settingsStore = writable<Settings | null>(null);
/**
 * 本进程曾检测到游戏处于独占全屏。放在设置页共享 store 中，
 * 即使当时正在其它分页或窗口隐藏，回到通用页也不会丢失提示。
 */
export const fullscreenWarningStore = writable(false);

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

/** 设置页通用的事务更新 + store 发布 + 用户反馈。 */
export async function patchSettings(
  patch: Record<string, unknown>,
  successMessage: string,
): Promise<boolean> {
  try {
    settingsStore.set(await updateSettings(patch));
    toast(true, successMessage);
    return true;
  } catch (error) {
    toast(false, `保存失败：${errText(error)}`);
    return false;
  }
}

export const toastInfo = (text: string): number => pushToast("info", text);
export const toastWarn = (text: string): number => pushToast("warning", text);

/**
 * 两段式提权提示的接力标记（localStorage）：
 * 第一段「以管理员权限重启」点击后写入，提权重启成功的新进程由
 * Settings.svelte 读取并弹出第二段「启动时自动请求管理员权限」询问。
 */
export const AUTO_ADMIN_PROMPT_FLAG = "kotone:auto-admin-prompt";

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  // Tauri 命令以结构化错误 reject 时（如 InjectError { message, needsElevation,
  // inputBlocked }），reject 值是普通对象而非 Error 实例——直接 String(e) 会得到
  // "[object Object]"（0.1.5 引导页「测试没有完成 [object Object]」用户反馈）。
  if (e !== null && typeof e === "object") {
    const msg = (e as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return msg;
    try {
      return JSON.stringify(e);
    } catch {
      /* fall through */
    }
  }
  return String(e);
}
