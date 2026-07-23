<script lang="ts">
  import { appState } from "../../lib/stores/state";
</script>

<!--
  悬浮录音条占位视图（docs/development.md §5.2 OverlayBar）。
  窗口本身 transparent / alwaysOnTop / skipTaskbar，这里只画一条圆角悬浮条。
-->
<div class="flex h-full items-center justify-center p-2">
  <div
    class="flex w-full items-center gap-3 rounded-2xl bg-kotone-deep/90 px-4 py-3 shadow-lg ring-1 ring-kotone-cyan/40"
  >
    <span class="inline-block h-3 w-3 rounded-full bg-kotone-cyan"></span>
    <div class="min-w-0">
      <p class="truncate text-sm font-medium text-white">Kotone 悬浮条（占位）</p>
      <p class="truncate text-xs text-white/60">
        当前状态：{$appState.state} · 波形 / 流式文本待实现
      </p>
    </div>
  </div>
</div>
