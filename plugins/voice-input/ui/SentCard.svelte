<script lang="ts">
  // 推送“发出去的话”（ADR 043 D10）：你向哪个频道发了什么。连着发的几句（同一个游戏、同一个频道、上一句发出后 2 分钟内）
  // 在同一张卡片里一句一行，原地更新；处理过的话和原话不一样时下面一行小字写原话。聊天里的卡片和点开以后右侧栏里是同一个。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { clockTime, type SentCardData } from "./pushes";

  let { data }: PluginViewProps = $props();
  const card = $derived(data && typeof data === "object" && "lines" in data ? (data as SentCardData) : null);
</script>

<div class="min-w-0" data-testid="voice-sent-card">
  {#if card}
    <p class="m-0 flex items-baseline gap-2 text-xs text-muted-foreground">
      <span class="min-w-0 flex-1 truncate" data-testid="voice-sent-to">{m.plugin_voice_input_card_sent_to({ channel: card.channel.name })}</span>
      <span class="shrink-0" data-testid="voice-sent-game">{card.game.name}</span>
    </p>
    <ul class="m-0 mt-1 list-none space-y-1 p-0">
      {#each card.lines as line, i (i)}
        <li class="min-w-0" data-testid="voice-sent-line">
          <div class="flex items-baseline gap-2">
            <span class="min-w-0 flex-1 break-words text-[13px] text-foreground">{line.text}</span>
            <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{clockTime(line.atMs)}</span>
          </div>
          {#if line.source}
            <p class="m-0 break-words text-[11px] text-muted-foreground" data-testid="voice-sent-source">{m.plugin_voice_input_card_source({ text: line.source })}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_voice_input_card_empty()}</p>
  {/if}
</div>
