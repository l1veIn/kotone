// 悬浮窗（ADR 043 D6，2026-11-26 晚修订）：游戏所在屏幕上的小窗。出现时角上一定是她的头像（外面一圈进度条），旁边是
// 气泡（一条简单的）或聊天小窗（复杂的：聊天页那段对话最近的几条，同一套组件）；旁边放哪个由页面按内容定
// （routes/floating/shape.ts）。后端 `floating/mod.rs` 管显示不显示、进度条、摆在哪、拖动、不抢焦点；这里是页面用的
// 命令和事件。浏览器预览里（Playwright）由 `@tauri-apps/api/mocks` 模拟这些命令和事件，没有模拟时什么都不显示。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri, type BubbleCorner } from "./ipc";
import type { PushCell } from "./agent";

export const FLOATING_EVENT = "kotone://floating";

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

/** 弹出聊天小窗的那条推送（新的卡片、带提醒的更新）：哪段对话、定位到哪一条（`插件:key`）。 */
export interface FloatingMini {
  conversationId: string;
  focus: string;
  key: string;
}

/**
 * 头像外面那一圈进度条（`floating::model::Ring`）：`atMs` 那一刻还剩 `leftMs`；`running` 时从那一刻起往下走，走完整个收起。
 * 停着（running = false）：鼠标在上面、拖着、她在听 / 在想 / 在说、等插件说结束。
 */
export interface FloatingRing {
  totalMs: number;
  leftMs: number;
  running: boolean;
  atMs: number;
}

/** 后端给的样子（`floating::Shown`）。 */
export interface FloatingView {
  /** 内容的版本。 */
  revision: number;
  shown: boolean;
  /** 她这一行在不在（内容是 kotone://bubble 的快照；她说完以后接着画她最后那句）。 */
  her: boolean;
  /** 推送的几行（新的在后面）。 */
  lines: FloatingLine[];
  mini: FloatingMini | null;
  /** 这一次出现的时间（毫秒）：分出“这一次来的”消息。 */
  since: number;
  ring: FloatingRing | null;
  /** 窗口贴着哪个角：头像在哪个角、内容往哪边长。 */
  corner: BubbleCorner;
  /** 聊天小窗最高多少（逻辑像素，游戏所在那块屏工作区高度的 40%）。 */
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

/** 一块挡鼠标的地方：`[x, y, 宽, 高]`（逻辑像素，相对整块内容的左上角）。 */
export type FloatingBox = [number, number, number, number];

/** 量好的内容大小（逻辑像素）：后端按它摆窗口、该显示就显示。`mini`：头像旁边放的是聊天小窗（进度条的一圈长一些）。
 *  `boxes`：头像、气泡 / 小窗各一块（带着阴影那一圈）；空数组表示已完成退场。 */
export async function floatingLayout(revision: number, width: number, height: number, mini: boolean, boxes: FloatingBox[]): Promise<void> {
  if (isTauri) await invoke("floating_layout", { revision, width, height, mini, boxes });
}

export async function floatingHover(inside: boolean): Promise<void> {
  if (isTauri) await invoke("floating_hover", { inside });
}

/** 玩家主动编辑快捷输入时借用键盘，离开后把焦点还给原窗口。 */
export async function floatingInputFocus(active: boolean): Promise<void> {
  if (isTauri) await invoke("floating_input_focus", { active });
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
