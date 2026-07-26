/*
 * 全局热键录入捕获的共享 helper（ADR-006）。
 * HotkeyPage 与首启向导共用：LL 钩子捕获下一个按键组合，结果经
 * kotone://hotkey-capture 事件推送；捕获期间全局热键匹配暂停，
 * Esc 由钩子层转成取消信号（不走 DOM keydown）。
 */

import { listen } from "@tauri-apps/api/event";
import {
  cancelHotkeyCapture,
  isTauri,
  startHotkeyCapture,
  type HotkeyCaptureEvent,
} from "./ipc";

export type CaptureResult =
  | { kind: "combo"; combo: string }
  | { kind: "cancelled" }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

/**
 * 启动一次热键捕获；结果恰好回调一次（含环境不支持的错误）。
 * 返回 cleanup：组件销毁 / 提前放弃时调用，兜底取消钩子侧捕获模式
 * （已结算后调用是 no-op）。
 */
export async function captureHotkey(
  onResult: (r: CaptureResult) => void,
): Promise<() => void> {
  if (!isTauri) {
    onResult({ kind: "error", message: "浏览器调试环境不支持热键录入" });
    return () => {};
  }
  let unlisten: (() => void) | undefined;
  let settled = false;
  const settle = (r: CaptureResult) => {
    if (settled) return;
    settled = true;
    unlisten?.();
    onResult(r);
  };
  try {
    unlisten = await listen<HotkeyCaptureEvent>("kotone://hotkey-capture", (ev) => {
      const p = ev.payload;
      if (p.combo) settle({ kind: "combo", combo: p.combo });
      else if (p.cancelled) settle({ kind: "cancelled" });
      else settle({ kind: "timeout" });
    });
    await startHotkeyCapture();
  } catch (e) {
    settle({ kind: "error", message: `无法启动热键录入：${errText(e)}` });
  }
  return () => {
    if (settled) return;
    settled = true;
    unlisten?.();
    void cancelHotkeyCapture();
  };
}
