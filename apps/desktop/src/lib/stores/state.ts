/*
 * UI 唯一数据源（docs/development.md §5.2 lib/stores/state.ts）。
 * 订阅 Rust 侧事件：
 *   "kotone://state"  { state, payload }  —— 状态机迁移（payload 携带文本/错误）
 *   "kotone://partial" { text }           —— 流式 partial；finalize 后也发一条最终文本
 *   "kotone://level"  { rms }             —— 录音 RMS 电平，驱动波形
 *
 * orchestrator 是唯一状态所有者（§4 设计要点 1），UI 不自行维护业务状态，
 * 这里只做事件 → store 的映射。浏览器（dev:web）环境下不监听事件，保持初值不炸。
 */

import { writable } from "svelte/store";
import {
  getSettings,
  isTauri,
  listSttEngines,
  type EngineInfo,
  type RuntimeStatus,
} from "../ipc";

export type KotoneState =
  | "idle"
  | "listening"
  | "transcribing"
  | "preview"
  | "sending"
  | "success"
  | "error";

export interface AppState {
  state: KotoneState;
  /** 流式引擎的 partial 文本（录音中实时上屏；非流式引擎为空） */
  partialText: string;
  /** 最终/预览文本（preview / sending / success 的 payload.text） */
  finalText: string;
  /** 录音 RMS 电平（0..1），驱动波形 */
  level: number;
  /** error 状态的消息 */
  errorMessage: string | null;
  /** error 状态保留的文本（发送失败可重试；可能为空） */
  errorText: string | null;
}

const initial: AppState = {
  state: "idle",
  partialText: "",
  finalText: "",
  level: 0,
  errorMessage: null,
  errorText: null,
};

export const appState = writable<AppState>({ ...initial });

// ---------- 事件 payload 类型 ----------

interface StateEventPayload {
  state: KotoneState;
  payload?: { text?: string | null; message?: string | null } | null;
}

interface PartialEventPayload {
  text: string;
}

interface LevelEventPayload {
  rms: number;
}

// ---------- 状态事件 → store 映射 ----------

function applyStateEvent(ev: StateEventPayload): void {
  appState.update((s) => {
    const next: AppState = { ...s, state: ev.state };
    const text = ev.payload?.text ?? null;
    const message = ev.payload?.message ?? null;

    switch (ev.state) {
      case "idle":
        // 会话彻底结束：清空所有瞬态
        next.partialText = "";
        next.finalText = "";
        next.level = 0;
        next.errorMessage = null;
        next.errorText = null;
        break;
      case "listening":
        // 新会话开始：清空上一轮残留
        next.partialText = text ?? "";
        next.finalText = "";
        next.errorMessage = null;
        next.errorText = null;
        break;
      case "transcribing":
        // 保留 partial 上屏内容，等待最终文本
        next.level = 0;
        break;
      case "preview":
      case "sending":
      case "success":
        if (text) {
          next.finalText = text;
          next.partialText = text;
        }
        break;
      case "error":
        next.errorMessage = message ?? "未知错误";
        if (text) next.errorText = text;
        break;
    }
    return next;
  });
}

// ---------- 事件订阅（Tauri 环境才启用） ----------

let unlistenAll: (() => void) | null = null;

/**
 * 订阅 Rust 侧三类事件；返回取消函数。
 * 非 Tauri 环境（浏览器 dev:web）直接返回 no-op，不抛错。
 * 重复调用安全（只订阅一次）。
 */
export async function initStateListeners(): Promise<() => void> {
  if (!isTauri) return () => {};
  if (unlistenAll) return unlistenAll;

  const { listen } = await import("@tauri-apps/api/event");
  const unlisteners = await Promise.all([
    listen<StateEventPayload>("kotone://state", (e) => applyStateEvent(e.payload)),
    listen<PartialEventPayload>("kotone://partial", (e) => {
      appState.update((s) => ({ ...s, partialText: e.payload.text }));
    }),
    listen<LevelEventPayload>("kotone://level", (e) => {
      appState.update((s) => ({ ...s, level: e.payload.rms }));
    }),
  ]);

  unlistenAll = () => {
    for (const u of unlisteners) u();
    unlistenAll = null;
  };
  return unlistenAll;
}

// ---------- 当前引擎是否流式（悬浮窗据此二选一渲染 partial 文本 / 声波） ----------

/** 当前 STT 引擎是否支持流式 partial（读取失败兜底 false = 按非流式渲染） */
export const engineStreaming = writable(false);

let streamingUnlisten: (() => void) | null = null;

/**
 * 初始化流式检测：拉取引擎清单 + 设置算出当前引擎的 streaming 能力，
 * 并监听 kotone://runtime（含 engineId）在引擎变化时重算。
 * 重复调用安全（只初始化一次）；返回取消函数。
 */
export async function initEngineStreaming(): Promise<() => void> {
  if (streamingUnlisten) return streamingUnlisten;

  let engines: EngineInfo[] = [];
  const apply = (engineId: string | null) => {
    engineStreaming.set(
      engines.find((e) => e.id === engineId)?.capabilities.streaming ?? false,
    );
  };

  try {
    const [list, settings] = await Promise.all([listSttEngines(), getSettings()]);
    engines = list;
    apply(settings.sttEngine);
  } catch {
    /* 读取失败保持 false（按非流式渲染） */
  }

  if (!isTauri) {
    streamingUnlisten = () => {
      streamingUnlisten = null;
    };
    return streamingUnlisten;
  }

  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<RuntimeStatus>("kotone://runtime", (e) => {
    if (e.payload.engineId) apply(e.payload.engineId);
  });
  streamingUnlisten = () => {
    un();
    streamingUnlisten = null;
  };
  return streamingUnlisten;
}
