/*
 * 鼠标跟随 spotlight（方向 B 悬停光晕的进阶件）：
 * 监听宿主 mousemove，把指针相对坐标写进 CSS 变量 --mx/--my；
 * 配合 app.css 的 .kotone-spotlight 类（::before 叠 radial-gradient）出光斑。
 * 只写 CSS 变量、只动 opacity/background，不触发布局；prefers-reduced-motion 下不注册。
 */
export function spotlight(node: HTMLElement): { destroy(): void } | void {
  if (
    typeof window === "undefined" ||
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
  ) {
    return;
  }
  const onMove = (e: MouseEvent) => {
    const r = node.getBoundingClientRect();
    node.style.setProperty("--mx", `${e.clientX - r.left}px`);
    node.style.setProperty("--my", `${e.clientY - r.top}px`);
  };
  node.addEventListener("mousemove", onMove, { passive: true });
  return {
    destroy() {
      node.removeEventListener("mousemove", onMove);
    },
  };
}
