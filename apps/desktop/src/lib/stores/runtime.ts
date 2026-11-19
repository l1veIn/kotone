/*
 * 语音输入状态的前端副本：
 * - 初始 getRuntimeStatus() 拉一次；
 * - 之后靠 kotone://runtime 事件全量推送对齐（start/stop 进度、
 *   引擎/模型切换导致的 restartNeeded 变化都由壳推送）。
 * 浏览器（dev:web）环境下用 ipc mock 的内存态，不监听事件。
 */

import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { getRuntimeStatus, isTauri, type RuntimeStatus } from "../ipc";
import { subscribeBrowserRuntime } from "../ipc/browserMock";

export const runtimeStore = writable<RuntimeStatus | null>(null);

export function voiceInputStatus(state: RuntimeStatus | null): string {
  if (!state) return "正在检查语音输入…";
  if (state.voiceInputPaused) return "语音输入已暂停";
  if (state.error) return "语音输入暂不可用";
  if (state.phase === "running") {
    if (state.restartNeeded) return "正在应用识别设置…";
    if (state.stage === "warmup") return "正在准备模型，可以按键录音";
    return "语音输入已就绪";
  }
  if (state.phase === "starting") return "正在准备语音输入…";
  if (state.phase === "stopping") return "正在释放语音输入资源…";
  return "语音输入待就绪";
}

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
  function accept(status: RuntimeStatus) {
    if (!active) return;
    revision++;
    runtimeStore.set(status);
  }
  unlisten = isTauri
    ? await listen<RuntimeStatus>("kotone://runtime", (event) => accept(event.payload))
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
