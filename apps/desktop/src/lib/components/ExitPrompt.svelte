<script lang="ts">
  /*
   * Kotone 自绘退出确认弹窗。只负责视觉与动作分发，
   * 具体的进程退出 / 隐藏到托盘由 Titlebar 执行。
   */
  import stickerRelax from "../../assets/brand/stickers/relax.webp";

  let {
    busy = false,
    onExit,
    onMinimize,
  }: {
    busy?: boolean;
    onExit: () => void;
    onMinimize: () => void;
  } = $props();
</script>

<div
  class="fixed inset-0 z-[80] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm"
  role="presentation"
>
  <div
    class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl w-full max-w-md p-6"
    role="dialog"
    aria-modal="true"
    aria-labelledby="exit-prompt-title"
  >
    <div class="flex items-start gap-4">
      <img src={stickerRelax} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 id="exit-prompt-title" class="text-base font-bold">要结束今天的演奏吗？</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
          彻底退出后，语音输入和全局快捷键都会停止。也可以先最小化到系统托盘，让琴音继续待命。
        </p>
      </div>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={onMinimize}
      >
        最小化到托盘
      </button>
      <button
        class="rounded-lg bg-brand-pink px-4 py-2 text-xs font-semibold text-white transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={onExit}
      >
        {busy ? "正在退出…" : "彻底退出"}
      </button>
    </div>
  </div>
</div>
