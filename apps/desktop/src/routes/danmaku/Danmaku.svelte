<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { isTauri } from "../../lib/ipc";
  import {
    DANMAKU_PREVIEW_EVENT, DANMAKU_PREVIEW_FINISHED_EVENT,
    danmakuMotion, danmakuStyle, danmakuText, danmakuTop, DEFAULT_DANMAKU_TOP_OFFSET_PX, type DanmakuPresentation,
  } from "../../lib/danmaku";

  let current = $state<DanmakuPresentation | null>(null);
  let lastId = -1;
  let expiry: ReturnType<typeof setTimeout> | undefined;
  const appearance = $derived(current ? danmakuStyle(current) : null);

  function complete(id: number) {
    if (current?.id !== id) return;
    current = null;
    clearTimeout(expiry);
    if (isTauri) void invoke("finish_danmaku", { id }).catch(() => {});
    else window.dispatchEvent(new CustomEvent(DANMAKU_PREVIEW_FINISHED_EVENT, { detail: id }));
  }

  function show(next: DanmakuPresentation | null) {
    if (!next) {
      clearTimeout(expiry);
      current = null;
      return;
    }
    // Snapshot + event delivery must not restart the same presentation, including after cancellation.
    if (next.id <= lastId) return;
    lastId = next.id;
    if (next.expiresAtMs <= Date.now()) return;
    clearTimeout(expiry);
    current = next;
    expiry = setTimeout(() => complete(next.id), next.expiresAtMs - Date.now());
  }

  function fly(node: HTMLElement, presentation: DanmakuPresentation) {
    let disposed = false;
    let animation: Animation | undefined;
    const reposition = () => {
      if (disposed) return;
      node.style.top = `${danmakuTop(window.innerHeight, node.getBoundingClientRect().height, presentation.topOffsetPx)}px`;
    };
    // Position can follow screen/DPI/font changes without restarting horizontal travel.
    reposition();
    window.addEventListener("resize", reposition);
    window.visualViewport?.addEventListener("resize", reposition);
    const sizeObserver = new ResizeObserver(reposition);
    sizeObserver.observe(node);
    void document.fonts.ready.then(() => {
      if (disposed || current?.id !== presentation.id) return;
      reposition();
      const motion = danmakuMotion(window.innerWidth, node.getBoundingClientRect().width, presentation.speedPxPerSec);
      const elapsed = Math.max(0, Date.now() - presentation.startedAtMs);
      if (elapsed >= motion.durationMs || presentation.expiresAtMs <= Date.now()) {
        complete(presentation.id);
        return;
      }
      node.dataset.durationMs = String(motion.durationMs);
      animation = node.animate([
        { transform: `translate3d(${motion.startX}px,0,0)` },
        { transform: `translate3d(${motion.endX}px,0,0)` },
      ], { duration: motion.durationMs, easing: "linear", fill: "both" });
      animation.currentTime = elapsed;
      void animation.finished.then(() => {
        if (!disposed) complete(presentation.id);
      }, () => {});
    });
    return { destroy() {
      disposed = true;
      window.removeEventListener("resize", reposition);
      window.visualViewport?.removeEventListener("resize", reposition);
      sizeObserver.disconnect();
      animation?.cancel();
    } };
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    if (isTauri) {
      void (async () => {
        let revision = 0;
        const stop = await listen<DanmakuPresentation | null>("kotone://danmaku", ({ payload }) => {
          revision += 1;
          if (!disposed) show(payload);
        });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const snapshotRevision = revision;
        const snapshot = await invoke<DanmakuPresentation | null>("get_danmaku");
        // An event during the request is newer, including a cancellation or a null snapshot.
        if (!disposed && revision === snapshotRevision) show(snapshot);
      })().catch(() => {});
    } else {
      const onPreview = (event: Event) => show((event as CustomEvent<DanmakuPresentation | null>).detail);
      window.addEventListener(DANMAKU_PREVIEW_EVENT, onPreview);
      unlisten = () => window.removeEventListener(DANMAKU_PREVIEW_EVENT, onPreview);
      show({ id: 1, text: "该打小龙了", startedAtMs: Date.now(), expiresAtMs: Date.now() + 60_000, speedPxPerSec: 160, fontSize: 32, topOffsetPx: DEFAULT_DANMAKU_TOP_OFFSET_PX, color: "#FFFFFF", emote: "none" });
    }
    return () => { disposed = true; unlisten?.(); clearTimeout(expiry); };
  });
</script>

<div class="stage" aria-live="polite" aria-atomic="true">
  {#if current && appearance}
    {#key current.id}
      <div class="message" data-testid="danmaku-message" data-presentation-id={current.id}
        style:font-size={`min(${appearance.fontSize}px, calc(100vh / 1.4))`} style:color={appearance.color} use:fly={current}>
        {#if appearance.emote.src}<img class="emote" src={appearance.emote.src} alt="" draggable="false" />{/if}
        <span>{danmakuText(current.text)}</span>
      </div>
    {/key}
  {/if}
</div>

<style>
  .stage { position: fixed; inset: 0; pointer-events: none; overflow: hidden; background: transparent; user-select: none; }
  .message { position: absolute; top: 0; left: 0; display: inline-flex; align-items: center; gap: .2em; width: max-content; max-width: none; padding: 0; margin: 0; border: 0; background: none; box-shadow: none; font-weight: 700; line-height: 1.35; white-space: nowrap; pointer-events: none; text-shadow: 1px 1px 2px #000, -1px -1px 2px #000, 0 2px 4px #0009; transform: translate3d(100vw, 0, 0); will-change: transform; }
  .emote { width: 1.4em; height: 1.4em; flex: 0 0 auto; object-fit: contain; pointer-events: none; }
</style>
