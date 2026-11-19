/*
 * 运行时「启动」状态的唯一前端副本（对齐 settingsStore 模式）：
 * - 初始 getRuntimeStatus() 拉一次；
 * - 之后靠 kotone://runtime 事件全量推送对齐（start/stop 进度、
 *   引擎/模型切换导致的 restartNeeded 变化都由壳推送）。
 * 浏览器（dev:web）环境下用 ipc mock 的内存态，不监听事件。
 */

import { derived, writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { getRuntimeStatus, isTauri, type RuntimeStatus } from "../ipc";
import { subscribeBrowserRuntime } from "../ipc/browserMock";
import { asrReadiness } from "../ipc/contracts";

export const runtimeStore = writable<RuntimeStatus | null>(null);

/** 语音识别就绪状态（ADR 028：只有本地识别才有「启动」）。 */
export const asrStore = derived(runtimeStore, (status) => asrReadiness(status));
/** 当前是本地识别：界面上才出现启动按钮、启动快捷键、全局状态。 */
export const asrLocalStore = derived(runtimeStore, (status) => status?.asrLocal ?? false);

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
  const accept = (status: RuntimeStatus) => {
    if (!active) return;
    revision++;
    runtimeStore.set(status);
  };
  unlisten = isTauri
    ? await listen<RuntimeStatus>("kotone://runtime", (e) => accept(e.payload))
    : subscribeBrowserRuntime(accept);
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
