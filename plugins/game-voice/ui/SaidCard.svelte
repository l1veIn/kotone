<script lang="ts">
  // 推送“她对队友说了”（ADR 043 D11）：她用游戏语音替你对队友说的话，和语音输入的“你向「队伍」发了”对称——凡是以你的
  // 名义对队友说的话，聊天里都留一条。上一句说完后 2 分钟内的几句在同一张卡片里一句一行，原地更新。
  // 聊天里的卡片和点开以后右侧栏里是同一个。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { SaidCardData } from "./voiceChat";

  let { data }: PluginViewProps = $props();
  const card = $derived(data && typeof data === "object" && "lines" in data ? (data as SaidCardData) : null);
  const clock = (atMs: number) => new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
</script>

<div class="min-w-0" data-testid="game-voice-said-card">
  {#if card}
    <p class="m-0 text-xs text-muted-foreground" data-testid="game-voice-said-to">{m.plugin_game_voice_card_said_to()}</p>
    <ul class="m-0 mt-1 list-none space-y-1 p-0">
      {#each card.lines as line, i (i)}
        <li class="flex min-w-0 items-baseline gap-2" data-testid="game-voice-said-line">
          <span class="min-w-0 flex-1 break-words text-[13px] text-foreground">{line.text}</span>
          <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{clock(line.atMs)}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_game_voice_card_empty()}</p>
  {/if}
</div>
