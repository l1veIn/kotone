import { writable } from "svelte/store";

/*
 * UI 唯一数据源（docs/development.md §5.2 lib/stores/state.ts）。
 * 占位实现：仅维护本地状态；后续订阅 Rust 侧事件：
 *   "kotone://state" / "kotone://partial" / "kotone://level" / "kotone://download"
 */

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
  /** 流式引擎的 partial 文本（非流式引擎为空） */
  partialText: string;
  /** 录音 RMS 电平，驱动波形 */
  level: number;
}

export const appState = writable<AppState>({
  state: "idle",
  partialText: "",
  level: 0,
});

// TODO: 在 Tauri 环境中 listen("kotone://state" | "kotone://partial" | "kotone://level")
// 并写入 appState；UI 不自行维护业务状态。
