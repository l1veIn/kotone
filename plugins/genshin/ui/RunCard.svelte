<script lang="ts">
  // 推送“BetterGI 的运行”（ADR 043 D11，后端 plugins/genshin/src/runs.rs）：一次运行一张卡片，开始时发、跑完 / 出错原地更新。
  // 聊天里的卡片和点开以后右侧栏里是同一个。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import RunLine from "./RunLine.svelte";
  import { isRun } from "./genshin";

  let { data }: PluginViewProps = $props();
  const run = $derived(isRun(data) ? data : null);
</script>

<div class="min-w-0" data-testid="genshin-run-card">
  {#if run}<RunLine {run} />{:else}<p class="m-0 text-xs text-muted-foreground">{m.plugin_genshin_card_empty()}</p>{/if}
</div>
