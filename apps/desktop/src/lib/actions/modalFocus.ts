const FOCUSABLE = "button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex='-1'])";

/** Focus a modal, keep keyboard navigation inside it, then restore the opening control. */
export function modalFocus(node: HTMLElement) {
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const controls = () => Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE))
    .filter((element) => element.getClientRects().length > 0 && !element.closest("[inert]"));
  const focusFirst = () => (controls()[0] ?? node).focus();
  queueMicrotask(() => { if (node.isConnected) focusFirst(); });
  const onKey = (event: KeyboardEvent) => {
    if (event.key !== "Tab") return;
    const items = controls();
    const first = items[0];
    const last = items[items.length - 1];
    if (!first) { event.preventDefault(); node.focus(); }
    else if (event.shiftKey && (document.activeElement === first || document.activeElement === node)) {
      event.preventDefault(); last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === node)) {
      event.preventDefault(); first.focus();
    }
  };
  node.addEventListener("keydown", onKey);
  return { destroy() {
    node.removeEventListener("keydown", onKey);
    if (previous?.isConnected) previous.focus();
  } };
}
