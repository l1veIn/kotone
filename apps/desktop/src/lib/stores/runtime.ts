/*
 * 运行时「启动」状态的唯一前端副本（对齐 settingsStore 模式）：
 * - 初始 getRuntimeStatus() 拉一次；
 * - 之后靠 kotone://runtime 事件全量推送对齐（start/stop 进度、
 *   引擎/模型切换导致的 restartNeeded 变化都由壳推送）。
 * 浏览器（dev:web）环境下用 ipc mock 的内存态，不监听事件。
 */

import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { getRuntimeStatus, isTauri, type RuntimeStatus } from "../ipc";

export const runtimeStore = writable<RuntimeStatus | null>(null);

let initialization: Promise<() => void> | null = null;

/**
 * 初始化 runtime store：拉取当前状态 + 订阅 kotone://runtime。
 * 重复调用安全（只订阅一次）；返回取消函数。
 */
export function initRuntime(): Promise<() => void> {
  if (!initialization) {
    initialization = subscribeRuntime().catch((error) => {
      initialization = null;
      throw error;
    });
  }
  return initialization;
}

async function subscribeRuntime(): Promise<() => void> {
  let revision = 0;
  let active = true;
  let unlisten: (() => void) | undefined;
  if (isTauri) {
    unlisten = await listen<RuntimeStatus>("kotone://runtime", (e) => {
      if (!active) return;
      revision++;
      runtimeStore.set(e.payload);
    });
  }
  const snapshotRevision = revision;
  try {
    const snapshot = await getRuntimeStatus();
    if (revision === snapshotRevision) runtimeStore.set(snapshot);
  } catch {
    /* 状态拉取失败保持 null，UI 按「未知」展示 */
  }
  return () => {
    if (!active) return;
    active = false;
    unlisten?.();
    initialization = null;
  };
}
