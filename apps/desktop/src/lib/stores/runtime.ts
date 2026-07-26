/*
 * 运行时「启动」状态的唯一前端副本（对齐 settingsStore 模式）：
 * - 初始 getRuntimeStatus() 拉一次；
 * - 之后靠 kotone://runtime 事件全量推送对齐（start/stop 进度、
 *   引擎/模型切换导致的 restartNeeded 变化都由壳推送）。
 * 浏览器（dev:web）环境下用 ipc mock 的内存态，不监听事件。
 */

import { writable } from "svelte/store";
import { getRuntimeStatus, isTauri, type RuntimeStatus } from "../ipc";

export const runtimeStore = writable<RuntimeStatus | null>(null);

let unlisten: (() => void) | null = null;

/**
 * 初始化 runtime store：拉取当前状态 + 订阅 kotone://runtime。
 * 重复调用安全（只订阅一次）；返回取消函数。
 */
export async function initRuntime(): Promise<() => void> {
  try {
    runtimeStore.set(await getRuntimeStatus());
  } catch {
    /* 状态拉取失败保持 null，UI 按「未知」展示 */
  }
  if (!isTauri) return () => {};
  if (unlisten) return unlisten;
  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<RuntimeStatus>("kotone://runtime", (e) => {
    runtimeStore.set(e.payload);
  });
  unlisten = () => {
    un();
    unlisten = null;
  };
  return unlisten;
}
