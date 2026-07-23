<script lang="ts">
  /*
   * 实时音量波形（docs/development.md §5.2 lib/components/Waveform.svelte）。
   * 由 Rust 侧 "kotone://level" 事件推送的 RMS 电平驱动（~50ms 一条）。
   * 电平静默时退化为缓慢呼吸动画，提示「仍在聆听」。
   */
  interface Props {
    /** RMS 电平（0..1，实际值通常远小于 1，内部做增益归一） */
    level?: number;
    /** 竖条数量 */
    bars?: number;
  }
  let { level = 0, bars = 16 }: Props = $props();

  // 耳麦 RMS 常见量级 0.02~0.3，做 4 倍增益并截断
  const norm = $derived(Math.min(1, Math.max(0, level * 4)));
  const quiet = $derived(norm < 0.03);

  /** 每根竖条高度（px）：底高 + 电平驱动的伪随机起伏 */
  function barHeight(i: number, n: number): number {
    const wobble = 0.35 + 0.65 * Math.abs(Math.sin(i * 7.13 + n * 23.7));
    return Math.round(3 + n * 25 * wobble);
  }
</script>

<div class="flex h-8 items-center gap-[3px]" class:quiet aria-label="录音电平波形">
  {#each Array(bars) as _, i}
    <span
      class="bar w-[3px] rounded-full"
      style:height="{barHeight(i, norm)}px"
      style:transition-delay="{i * 6}ms"
    ></span>
  {/each}
</div>

<style>
  .bar {
    background: linear-gradient(to top, #00e5ff, #7b2fff);
    opacity: 0.9;
    transition: height 90ms ease-out;
  }
  /* 静默时整体缓慢呼吸，表明仍在聆听 */
  .quiet .bar {
    animation: kotone-breathe 1.6s ease-in-out infinite;
  }
  .quiet .bar:nth-child(2n) {
    animation-delay: 0.4s;
  }
  .quiet .bar:nth-child(3n) {
    animation-delay: 0.8s;
  }
  @keyframes kotone-breathe {
    0%,
    100% {
      opacity: 0.35;
      transform: scaleY(0.7);
    }
    50% {
      opacity: 0.9;
      transform: scaleY(1);
    }
  }
</style>
