// 消息提示（ADR 039 D4）：发自动卡片时屏幕右下角的小窗。后端 `message_popup.rs` 管窗口（不抢焦点、不进截图、
// 游戏独占全屏时不弹）；这里是小窗页面用的命令和事件。浏览器预览里给一条示例，点它只记一笔（e2e 看）。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "./ipc";

export interface MessagePopup {
  plugin: string;
  pluginName: string;
  /** 她现在叫什么。 */
  name: string;
  /** 插件给的那一句（≤ 24 字）。 */
  text: string;
  conversationId: string;
  key: string;
  revision: number;
}

/** 浏览器预览：一条示例；点开 / 关掉记在 `window.messagePopupTest` 上。 */
interface PopupTest { popup: MessagePopup | null; opened: number[]; closed: number[] }
function previewHook(): PopupTest {
  const w = window as unknown as { messagePopupTest?: PopupTest };
  // i18n-ignore-start：浏览器预览的示例数据（真实应用里来自插件），不是界面文字
  w.messagePopupTest ??= {
    popup: { plugin: "lol", pluginName: "英雄联盟", name: "琴音", text: "对面的战绩到了", conversationId: "c-preview", key: "opening:1", revision: 1 },
    opened: [], closed: [],
  };
  // i18n-ignore-end
  return w.messagePopupTest;
}

export async function messagePopupCurrent(): Promise<MessagePopup | null> {
  if (!isTauri) return previewHook().popup;
  return (await invoke<MessagePopup | null>("message_popup_current")) ?? null;
}

export function subscribeMessagePopup(callback: (popup: MessagePopup) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<MessagePopup>("kotone://message-popup", (e) => callback(e.payload));
}

/** 到点了 / 点了 ×：收起这一条（新的已经换上来就不动）。 */
export async function messagePopupClose(revision: number): Promise<void> {
  if (!isTauri) { previewHook().closed.push(revision); return; }
  await invoke("message_popup_close", { revision });
}

/** 点了小窗：打开 Kotone 的聊天页，定位到那张卡片。 */
export async function messagePopupOpen(revision: number): Promise<void> {
  if (!isTauri) { previewHook().opened.push(revision); return; }
  await invoke("message_popup_open", { revision });
}
