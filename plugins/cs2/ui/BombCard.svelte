<script lang="ts">
  // 下包倒数（推送 `bomb`，`ui://kotone/cs2/bomb`）：下了以后从 explodesAtMs 自己倒数（不每秒推一次），拆了、炸了、回合结束时
  // 插件原地更新这张卡片（同一个 key）。只说事实：还剩几秒、拆了、炸了。
  import { onDestroy } from "svelte";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { bombLeft, mapLabel, type BombView } from "./cs2";

  let { data }: PluginViewProps = $props();
  const bomb = $derived(data && typeof data === "object" && "explodesAtMs" in data ? (data as BombView) : null);
  let now = $state(Date.now());
  const timer = setInterval(() => (now = Date.now()), 250);
  onDestroy(() => clearInterval(timer));
  const ticking = $derived(bomb?.state === "planted" && now < (bomb?.explodesAtMs ?? 0));
  const left = $derived(bomb ? bombLeft(bomb, now) : 0);
  const total = $derived(bomb ? Math.max(1, (bomb.explodesAtMs - bomb.plantedAtMs) / 1000) : 40);
  const stateText = $derived(bomb?.state === "defused" ? m.plugin_cs2_bomb_defused() : bomb?.state === "exploded" ? m.plugin_cs2_bomb_exploded()
    : bomb?.state === "over" ? m.plugin_cs2_bomb_over() : m.plugin_cs2_bomb_planted());
</script>

<div class="min-w-0" data-testid="cs2-bomb-card" data-state={bomb?.state ?? ""}>
  {#if bomb}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_cs2_bomb_round({ round: bomb.round, map: mapLabel(bomb.map, bomb.mapName) })}</p>
    <div class="mt-1 flex items-baseline gap-3">
      <span class="text-[13px] font-medium text-foreground" data-testid="cs2-bomb-state">{stateText}</span>
      {#if ticking}
        <span class="text-[22px] font-semibold tabular-nums {left <= 10 ? 'text-rose-600 dark:text-rose-300' : 'text-foreground'}"
          data-testid="cs2-bomb-left">{m.plugin_cs2_bomb_seconds({ seconds: left })}</span>
      {/if}
    </div>
    {#if ticking}
      <div class="mt-1.5 h-1 overflow-hidden rounded bg-muted" aria-hidden="true">
        <div class="h-full rounded {left <= 10 ? 'bg-rose-500' : 'bg-amber-500'}" style:width={`${Math.min(100, (left / total) * 100)}%`}></div>
      </div>
    {/if}
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_cs2_bomb_empty()}</p>
  {/if}
</div>
