<script lang="ts">
  import { appState } from "../../lib/stores/state";
</script>

<!--
  设置窗口占位视图（docs/development.md §5.2 routes/settings）。
  后续包含：交互模式与热键 / 麦克风 / STT 引擎切换与模型管理 / 游戏 profile / 评测导出。
-->
<div class="h-full overflow-auto bg-kotone-deep text-white">
  <div class="mx-auto max-w-xl p-8">
    <h1 class="text-2xl font-bold text-kotone-cyan">Kotone 设置</h1>
    <p class="mt-2 text-sm text-white/60">设置界面占位 —— 业务配置项待后续实现。</p>

    <ul class="mt-6 space-y-3 text-sm">
      <li class="rounded-lg bg-white/5 p-3 ring-1 ring-white/10">交互模式与热键（hold / toggle）</li>
      <li class="rounded-lg bg-white/5 p-3 ring-1 ring-white/10">麦克风设备选择</li>
      <li class="rounded-lg bg-white/5 p-3 ring-1 ring-white/10">STT 引擎切换与模型管理</li>
      <li class="rounded-lg bg-white/5 p-3 ring-1 ring-white/10">游戏 profile 编辑器</li>
      <li class="rounded-lg bg-white/5 p-3 ring-1 ring-white/10">评测数据导出</li>
    </ul>

    <p class="mt-6 text-xs text-white/40">当前状态：{$appState.state}</p>
  </div>
</div>
