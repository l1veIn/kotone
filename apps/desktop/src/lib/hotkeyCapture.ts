/*
 * 全局热键录入捕获的共享 helper（ADR-006）。
 * HotkeyPage 与首启向导共用：LL 钩子捕获下一个键盘组合或鼠标侧键，结果经
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
import { keyboardEventCombo, mouseEventCombo } from "./hotkeyCombo";

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
  options: { signal?: AbortSignal } = {},
): Promise<() => void> {
  if (options.signal?.aborted) return () => {};
  if (captureActive) {
    onResult({ kind: "error", message: "已有热键录入进行中，请先完成或按 Esc 取消" });
    return () => {};
  }
  captureActive = true;
  let unlisten: (() => void) | undefined;
  let settled = false;
  let backendStarted = false;
  let backendRequested = false;
  let startup: Promise<void> = Promise.resolve();
  let finishing: Promise<void> | undefined;
  const abort = () => { void finish(undefined, true); };
  const cleanupWebview = () => {
    window.removeEventListener("keydown", blockWebviewKey, { capture: true });
    window.removeEventListener("keyup", blockWebviewKey, { capture: true });
    window.removeEventListener("mousedown", blockWebviewMouse, { capture: true });
    options.signal?.removeEventListener("abort", abort);
  };
  const finish = (result: CaptureResult | undefined, stopBackend: boolean): Promise<void> => {
    if (finishing) return finishing;
    settled = true;
    unlisten?.();
    cleanupWebview();
    // 安装中的录入也必须完成清理后才能让下一位取得所有权，否则旧 cancel
    // 可能关闭新页面的捕获。AbortSignal 可在 listen/start 尚未返回时取消。
    finishing = (async () => {
      await startup;
      if (stopBackend && backendRequested) {
        try { await cancelHotkeyCapture(); }
        catch (error) {
          if (result) result = { kind: "error", message: `结束热键录入失败：${errText(error)}` };
        }
      }
      captureActive = false;
      if (result && !options.signal?.aborted) onResult(result);
    })();
    return finishing;
  };
  const finishFromWebview = (result: CaptureResult) => {
    if (!backendStarted || settled) return;
    // 先停止监听并标记结算，避免 cancel 产生的后端 Cancelled 事件覆盖窗口兜底结果；
    // 等后端确实退出 capture 后再通知调用页保存热键，避免保存触发的 hook 重注册
    // 与尚未清理的 capture 槽发生竞态。
    void finish(result, true);
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
  const blockWebviewMouse = (event: MouseEvent) => {
    const combo = mouseEventCombo(event);
    if (!combo) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (isTauri && !backendStarted) return;
    if (isTauri) {
      void logFrontendError(
        "hotkey-capture-fallback",
        `低级鼠标钩子未捕获本次侧键，已通过设置窗口兜底录入：${combo}`,
      ).catch(() => {});
    }
    finishFromWebview({ kind: "combo", combo });
  };
  window.addEventListener("keydown", blockWebviewKey, { capture: true });
  window.addEventListener("keyup", blockWebviewKey, { capture: true });
  window.addEventListener("mousedown", blockWebviewMouse, { capture: true });
  options.signal?.addEventListener("abort", abort, { once: true });
  if (!isTauri) {
    // dev:web / E2E 直接使用 WebView 键盘兜底，既便于回归向导流程，也与
    // Windows 底层 hook 失活时的用户可见行为保持一致。
    backendStarted = true;
    return abort;
  }
  startup = (async () => {
    try {
      unlisten = await listen<HotkeyCaptureEvent>("kotone://hotkey-capture", (ev) => {
        const p = ev.payload;
        const result: CaptureResult = p.combo ? { kind: "combo", combo: p.combo }
          : p.cancelled ? { kind: "cancelled" } : { kind: "timeout" };
        void finish(result, false);
      });
      if (settled) { unlisten(); return; }
      backendRequested = true;
      await startHotkeyCapture();
      backendStarted = true;
    } catch (error) {
      void finish({ kind: "error", message: `无法启动热键录入：${errText(error)}` }, backendRequested);
    }
  })();
  await startup;
  if (settled) await finishing;
  return abort;
}
