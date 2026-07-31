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
  logFrontendError,
  startHotkeyCapture,
  type HotkeyCaptureEvent,
} from "./ipc";
import { keyboardEventCombo } from "./hotkeyCombo";

export type CaptureResult =
  | { kind: "combo"; combo: string }
  | { kind: "cancelled" }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

let captureActive = false;

/** 供窗口内热键兜底判断：录入期间不能把同一个按键当成正式热键触发。 */
export function isHotkeyCaptureActive(): boolean {
  return captureActive;
}

/**
 * 启动一次热键捕获；结果恰好回调一次（含环境不支持的错误）。
 * 返回 cleanup：组件销毁 / 提前放弃时调用，兜底取消钩子侧捕获模式
 * （已结算后调用是 no-op）。
 */
export async function captureHotkey(
  onResult: (r: CaptureResult) => void,
): Promise<() => void> {
  let unlisten: (() => void) | undefined;
  let settled = false;
  let backendStarted = false;
  const finishFromWebview = (result: CaptureResult) => {
    if (!backendStarted || settled) return;
    // 先停止监听并标记结算，避免 cancel 产生的后端 Cancelled 事件覆盖窗口兜底结果；
    // 等后端确实退出 capture 后再通知调用页保存热键，避免保存触发的 hook 重注册
    // 与尚未清理的 capture 槽发生竞态。
    settled = true;
    unlisten?.();
    cleanupWebview();
    void cancelHotkeyCapture()
      .catch(() => {})
      .finally(() => onResult(result));
  };
  const blockWebviewKey = (event: KeyboardEvent) => {
    // 修饰键仍允许浏览器维护 modifier 状态；真正被录入的主键（含 Esc）不得
    // 触发 Tab 换焦点、Space 点击按钮或 Enter 提交等页面默认行为。
    if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.type !== "keydown" || event.repeat) return;

    // 后端安装/刷新钩子和执行合成 F24 预检期间不接受用户输入；否则用户在这
    // 250ms 内抢按会绕过尚未开启的 capture 槽，造成“钩子不可用”的假阳性。
    if (isTauri && !backendStarted) return;

    if (event.key === "Escape") {
      finishFromWebview({ kind: "cancelled" });
      return;
    }
    const combo = keyboardEventCombo(event);
    if (combo) {
      if (isTauri) {
        // 到达 WebView 只能证明这一事件没有被 LL capture 吞掉，不能单凭一次事件
        // 归因于安全软件：hook 线程刚刷新、键盘重映射产生 injected 事件等也会如此。
        // 保留窗口内兜底完成录入，并把事实写进诊断日志；真正的拦截预警由独立
        // SendInput 环境自检负责。
        void logFrontendError(
          "hotkey-capture-fallback",
          `低级键盘钩子未捕获本次按键，已通过设置窗口兜底录入：${combo}`,
        ).catch(() => {});
      }
      finishFromWebview({ kind: "combo", combo });
    }
  };
  captureActive = true;
  window.addEventListener("keydown", blockWebviewKey, { capture: true });
  window.addEventListener("keyup", blockWebviewKey, { capture: true });

  const cleanupWebview = () => {
    captureActive = false;
    window.removeEventListener("keydown", blockWebviewKey, { capture: true });
    window.removeEventListener("keyup", blockWebviewKey, { capture: true });
  };
  const settle = (r: CaptureResult) => {
    if (settled) return;
    settled = true;
    unlisten?.();
    cleanupWebview();
    onResult(r);
  };
  if (!isTauri) {
    // dev:web / E2E 直接使用 WebView 键盘兜底，既便于回归向导流程，也与
    // Windows 底层 hook 失活时的用户可见行为保持一致。
    backendStarted = true;
    return () => {
      if (settled) return;
      settled = true;
      cleanupWebview();
    };
  }
  try {
    unlisten = await listen<HotkeyCaptureEvent>("kotone://hotkey-capture", (ev) => {
      const p = ev.payload;
      if (p.combo) settle({ kind: "combo", combo: p.combo });
      else if (p.cancelled) settle({ kind: "cancelled" });
      else settle({ kind: "timeout" });
    });
    await startHotkeyCapture();
    backendStarted = true;
  } catch (e) {
    settle({ kind: "error", message: `无法启动热键录入：${errText(e)}` });
  }
  return () => {
    if (settled) return;
    settled = true;
    unlisten?.();
    cleanupWebview();
    void cancelHotkeyCapture();
  };
}
