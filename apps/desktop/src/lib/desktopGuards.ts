import { isTauri } from "./ipc";

/**
 * 桌面壳不是浏览器：阻止 WebView2 的页面级菜单和导航快捷键。
 *
 * Windows 侧还会关闭 WebView2 browser accelerator keys；这里保留捕获阶段的
 * 防线，覆盖嵌入式 WebView 行为差异，并避免快捷键落到页面组件。
 */
export function installDesktopGuards() {
  if (!isTauri) return;
  // 开发构建（pnpm dev）保留右键菜单与开发者工具，便于调试报错
  if (import.meta.env.DEV) return;

  window.addEventListener(
    "contextmenu",
    (event) => {
      event.preventDefault();
    },
    { capture: true },
  );

  window.addEventListener(
    "keydown",
    (event) => {
      const key = event.key.toLowerCase();
      const browserShortcut =
        key === "f5" ||
        key === "f7" ||
        key === "f12" ||
        (event.ctrlKey &&
          (key === "f" ||
            key === "r" ||
            key === "u" ||
            key === "p" ||
            key === "s" ||
            key === "o" ||
            key === "l" ||
            (event.shiftKey && ["i", "j", "c", "r"].includes(key))));
      if (!browserShortcut) return;
      event.preventDefault();
      event.stopImmediatePropagation();
    },
    { capture: true },
  );
}
