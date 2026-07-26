<script lang="ts">
  /*
   * 右上角 toast 堆叠（方向 B 自绘，不引库）：
   * - 四种类型：success 青 / info 白 / warning 黄 / error 品红，各配图标与光晕；
   * - 滑入滑出动画（prefers-reduced-motion 降级为淡入淡出）；
   * - 自动消隐时长由 store 控制（success/info/warning 4s，error 8s），可点 × 立即关。
   */
  import { fade, fly } from "svelte/transition";
  import { toasts, dismissToast, type ToastKind } from "../stores/ui";

  const reduced =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;

  /** 正常：右侧滑入滑出；reduced-motion：短淡入淡出 */
  function toastTrans(node: HTMLElement) {
    return reduced ? fade(node, { duration: 120 }) : fly(node, { x: 48, duration: 220 });
  }

  const meta: Record<ToastKind, { icon: string; cls: string }> = {
    success: { icon: "✓", cls: "text-kotone-cyan ring-kotone-cyan/45 shadow-glow-cyan" },
    info: { icon: "ℹ", cls: "text-white/85 ring-white/25" },
    warning: {
      icon: "⚠",
      cls: "text-yellow-300 ring-yellow-400/45 shadow-[0_0_14px_rgba(250,204,21,0.25)]",
    },
    error: { icon: "✕", cls: "text-kotone-pink ring-kotone-pink/55 shadow-glow-pink" },
  };
</script>

<!-- top-16：让开 64px 标题栏；z-[70]：压过首启向导（z-50） -->
<div class="pointer-events-none fixed top-16 right-4 z-[70] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2">
  {#each $toasts as t (t.id)}
    <div
      transition:toastTrans
      role="status"
      class="pointer-events-auto flex items-start gap-2.5 rounded-xl bg-kotone-panel/92 px-3.5 py-2.5 ring-1 backdrop-blur {meta[t.kind].cls}"
    >
      <span class="mt-px shrink-0 text-sm leading-none font-bold" aria-hidden="true">
        {meta[t.kind].icon}
      </span>
      <p class="min-w-0 flex-1 text-[12px] leading-relaxed break-words">{t.text}</p>
      <button
        class="shrink-0 rounded px-1 text-white/40 transition hover:bg-white/10 hover:text-white/85"
        title="关闭"
        aria-label="关闭提示"
        onclick={() => dismissToast(t.id)}
      >
        ×
      </button>
    </div>
  {/each}
</div>
