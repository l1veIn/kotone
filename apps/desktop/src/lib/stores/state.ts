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
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "../ipc";

export type KotoneState =
  | "idle"
  | "listening"
  | "transcribing"
  | "processing"
  | "preview"
  | "sending"
  | "success"
  | "error";

export interface ChannelInfo {
  id: string;
  displayName: string;
  isDefault: boolean;
}

export interface ProcessingStepInfo {
  stepId: string;
  processorId: string;
  displayName: string;
  index: number;
  total: number;
}

export interface AppState {
  state: KotoneState;
  /** 流式引擎的 partial 文本（录音中实时上屏；非流式引擎为空） */
  partialText: string;
  /** 最终/预览文本（preview / sending / success 的 payload.text） */
  finalText: string;
  /** 录音 RMS 电平（0..1），驱动波形 */
  level: number;
  /** listening 状态的可恢复降级提示（例如 VAD 不可用时改为手动结束） */
  warningMessage: string | null;
  /** error 状态的消息 */
  errorMessage: string | null;
  /** error 状态保留的文本（发送失败可重试；可能为空） */
  errorText: string | null;
  /** error 是否为「模拟输入被系统拦截」（安全软件/反作弊）；true 时弹排查引导弹窗 */
  inputBlocked: boolean;
  /** error 是否为 UIPI 提权问题（目标程序以更高权限运行）；true 时提示管理员运行 */
  needsElevation: boolean;
  /** 当前聊天频道（多频道 profile 才有值；切回默认时为 null 让悬浮窗隐藏徽标） */
  channel: ChannelInfo | null;
  /** Processing 状态下正在执行的后处理步骤；尚未进入具体步骤时为 null。 */
  processingStep: ProcessingStepInfo | null;
}

const initial: AppState = {
  state: "idle",
  partialText: "",
  finalText: "",
  level: 0,
  warningMessage: null,
  errorMessage: null,
  errorText: null,
  inputBlocked: false,
  needsElevation: false,
  channel: null,
  processingStep: null,
};

export const appState = writable<AppState>({ ...initial });

// ---------- 事件 payload 类型 ----------

interface StateEventPayload {
  state: KotoneState;
  payload?: {
    text?: string | null;
    message?: string | null;
    warning?: string | null;
    preserveText?: boolean | null;
    inputBlocked?: boolean | null;
    needsElevation?: boolean | null;
    stepId?: string | null;
    processorId?: string | null;
    displayName?: string | null;
    index?: number | null;
    total?: number | null;
  } | null;
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
    const warning = ev.payload?.warning ?? null;
    if (ev.state !== "processing") next.processingStep = null;
    if (ev.state !== "listening") next.warningMessage = null;

    switch (ev.state) {
      case "idle":
        // 会话彻底结束：清空所有瞬态
        next.partialText = "";
        next.finalText = "";
        next.level = 0;
        next.warningMessage = null;
        next.errorMessage = null;
        next.errorText = null;
        next.inputBlocked = false;
        next.needsElevation = false;
        break;
      case "listening":
        // 新会话开始：清空上一轮残留
        next.partialText = text ?? (ev.payload?.preserveText === true ? s.partialText : "");
        next.finalText = "";
        next.warningMessage = warning;
        next.errorMessage = null;
        next.errorText = null;
        next.inputBlocked = false;
        next.needsElevation = false;
        break;
      case "transcribing":
        // 保留 partial 上屏内容，等待最终文本
        next.level = 0;
        break;
      case "processing":
        // 后处理展示识别原文；完成后 Preview/Sending 的 payload 会替换为可交付文本。
        next.level = 0;
        if (text) next.partialText = text;
        next.processingStep =
          ev.payload?.stepId &&
          ev.payload.processorId &&
          ev.payload.displayName &&
          typeof ev.payload.index === "number" &&
          typeof ev.payload.total === "number"
            ? {
                stepId: ev.payload.stepId,
                processorId: ev.payload.processorId,
                displayName: ev.payload.displayName,
                index: ev.payload.index,
                total: ev.payload.total,
              }
            : null;
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
        next.inputBlocked = ev.payload?.inputBlocked === true;
        next.needsElevation = ev.payload?.needsElevation === true;
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

  const unlisteners = await Promise.all([
    listen<StateEventPayload>("kotone://state", (e) => applyStateEvent(e.payload)),
    listen<PartialEventPayload>("kotone://partial", (e) => {
      appState.update((s) => ({ ...s, partialText: e.payload.text }));
    }),
    listen<LevelEventPayload>("kotone://level", (e) => {
      appState.update((s) => ({ ...s, level: e.payload.rms }));
    }),
    listen<ChannelInfo>("kotone://channel", (e) => {
      // 默认频道不挂徽标：存 null
      appState.update((s) => ({
        ...s,
        channel: e.payload.isDefault ? null : e.payload,
      }));
    }),
  ]);

  unlistenAll = () => {
    for (const u of unlisteners) u();
    unlistenAll = null;
  };
  return unlistenAll;
}
