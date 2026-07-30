<script lang="ts">
  /*
   * 右上角 toast 堆叠（方向 B 自绘，不引库）：
   * - 四种类型：success 青 / info 白 / warning 黄 / error 品红，各配人物贴纸与光晕；
   * - 滑入滑出动画（prefers-reduced-motion 降级为淡入淡出）；
   * - 自动消隐时长由 store 控制（success/info/warning 4s，error 8s），可点 × 立即关。
   */
  import { fade, fly } from "svelte/transition";
  import { toasts, dismissToast, type ToastKind } from "../stores/ui";
  import stickerProud from "../../assets/brand/stickers/proud.webp";
  import stickerPointing from "../../assets/brand/stickers/pointing.webp";
  import stickerThinking from "../../assets/brand/stickers/thinking.webp";
  import stickerAmazed from "../../assets/brand/stickers/amazed.webp";

  const reduced =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;

  /** 正常：右侧滑入滑出；reduced-motion：短淡入淡出 */
  function toastTrans(node: HTMLElement) {
    return reduced ? fade(node, { duration: 120 }) : fly(node, { x: 48, duration: 220 });
  }

  /** 每种 toast 配一张人物贴纸：成功=得意 / 信息=指引 / 警告=思考 / 错误=惊讶 */
  const meta: Record<ToastKind, { icon: string; cls: string; sticker: string }> = {
    success: { icon: "✓", cls: "text-kotone-cyan ring-kotone-cyan/45 shadow-glow-cyan", sticker: stickerProud },
    info: { icon: "ℹ", cls: "text-white/85 ring-white/25", sticker: stickerPointing },
    warning: {
      icon: "⚠",
      cls: "text-yellow-300 ring-yellow-400/45 shadow-[0_0_14px_rgba(250,204,21,0.25)]",
      sticker: stickerThinking,
    },
    error: { icon: "✕", cls: "text-kotone-pink ring-kotone-pink/55 shadow-glow-pink", sticker: stickerAmazed },
  };
</script>

<!-- top-13：让开 48px 标题栏；z-[70]：压过首启向导（z-50） -->
<div class="pointer-events-none fixed top-13 right-4 z-[70] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2">
  {#each $toasts as t (t.id)}
    <div
      transition:toastTrans
      role="status"
      class="pointer-events-auto flex items-center gap-3 rounded-xl bg-kotone-panel/92 px-4 py-3 ring-1 backdrop-blur {meta[t.kind].cls}"
    >
      <img
        src={meta[t.kind].sticker}
        alt=""
        class="h-11 w-11 shrink-0 object-contain"
        aria-hidden="true"
      />
      <p class="min-w-0 flex-1 text-[13px] leading-relaxed break-words">{t.text}</p>
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
