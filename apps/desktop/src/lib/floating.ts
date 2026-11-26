// 悬浮窗（ADR 043 D6）：游戏所在屏幕上的小窗，两种状态——气泡（她在听、在想、在说，或插件推送的一行）和小窗（这段对话里的
// 推送卡片，和聊天页同一种组件）。后端 `floating/mod.rs` 管显示什么、什么时候收、摆在哪、拖动、不抢焦点；这里是页面用的
// 命令和事件。浏览器预览里（Playwright）由 `@tauri-apps/api/mocks` 模拟这些命令和事件，没有模拟时什么都不显示。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri, type BubbleCorner } from "./ipc";
import type { PushCell } from "./agent";

export const FLOATING_EVENT = "kotone://floating";

export type FloatingMode = "hidden" | "bubble" | "mini";

/** 气泡里的一行推送。 */
export interface FloatingLine {
  /** `插件:key`。 */
  id: string;
  plugin: string;
  pluginName: string;
  key: string;
  conversationId: string;
  cells: PushCell[];
  /** 一直显示到插件说结束（三选一）。 */
  untilDone: boolean;
}

/** 弹出的小窗：哪段对话、定位到哪一条（`插件:key`，小窗的高度跟着它）。 */
export interface FloatingMini {
  conversationId: string;
  focus: string;
  key: string;
}

/** 后端给的样子（`floating::Shown`）。 */
export interface FloatingView {
  revision: number;
  mode: FloatingMode;
  /** 她这一行显示不显示（内容是 kotone://bubble 的快照）。 */
  her: boolean;
  lines: FloatingLine[];
  mini: FloatingMini | null;
  /** 窗口贴着哪个角：内容往哪边靠、从哪边长。 */
  corner: BubbleCorner;
  /** 小窗最高多少（逻辑像素，游戏所在那块屏工作区高度的 40%）。 */
  maxHeight: number;
}

export async function floatingCurrent(): Promise<FloatingView | null> {
  if (!isTauri) return null;
  return (await invoke<FloatingView | null>("floating_current")) ?? null;
}

export function subscribeFloating(callback: (view: FloatingView) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<FloatingView>(FLOATING_EVENT, (e) => callback(e.payload));
}

/** 量好的内容大小（逻辑像素）：后端按它摆窗口、该显示就显示。 */
export async function floatingLayout(revision: number, width: number, height: number): Promise<void> {
  if (isTauri) await invoke("floating_layout", { revision, width, height });
}

export async function floatingHover(inside: boolean): Promise<void> {
  if (isTauri) await invoke("floating_hover", { inside });
}

/** ×：现在显示着的都收起，直到下一条推送。 */
export async function floatingClose(): Promise<void> {
  if (isTauri) await invoke("floating_close");
}

/** 按下左键：后端跟着光标挪窗口（不激活），松开记下位置。只是点一下不动它。 */
export async function floatingDrag(): Promise<void> {
  if (isTauri) await invoke("floating_drag");
}

/** 设置页点了“位置还原”：照现在的设置重摆一次。 */
export async function floatingReposition(): Promise<void> {
  if (isTauri) await invoke("floating_reposition");
}

/** 打开 Kotone 的聊天页、定位到这条推送（玩家自己的动作，主窗口拿到前面）。 */
export async function floatingOpen(conversationId: string, key: string): Promise<void> {
  if (isTauri) await invoke("auto_card_open", { conversationId, key });
}
