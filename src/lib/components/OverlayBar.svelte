<script lang="ts">
  /*
   * 悬浮录音条组件占位（docs/development.md §5.2）。
   * 后续职责：波形动画、流式 partial 文本区、状态指示、结果预览与编辑。
   * 当前由 routes/overlay/Overlay.svelte 内联占位替代，接入后替换。
   */
  import { appState } from "../stores/state";
</script>

<div class="flex w-full items-center gap-3 rounded-2xl bg-kotone-deep/90 px-4 py-3">
  <span class="inline-block h-3 w-3 rounded-full bg-kotone-cyan"></span>
  <p class="text-sm text-white">OverlayBar 占位 · {$appState.state}</p>
</div>
