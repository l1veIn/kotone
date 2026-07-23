<script lang="ts">
  /*
   * 实时音量波形占位（docs/development.md §5.2）。
   * 后续由 Rust 侧 "kotone://level" 事件推送 RMS 电平驱动动画。
   */
  interface Props {
    level?: number;
  }
  let { level = 0 }: Props = $props();
</script>

<div class="flex h-6 items-center gap-1" aria-label="waveform placeholder">
  {#each Array(12) as _, i}
    <span
      class="w-1 rounded-full bg-kotone-cyan/70"
      style:height="{4 + ((i * 7 + Math.round(level * 10)) % 12)}px"
    ></span>
  {/each}
</div>
