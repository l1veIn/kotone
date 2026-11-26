<script lang="ts">
  // 推送“这局的同传”（ADR 043 D11）：一局一张卡片，同传每翻好一句原地加一行——译文在上，听到的原话一行小字在下，新的在下面。
  // 卡片里只留最近的几句（再早的在后台那段对话里），上面写着一共几句、译成什么。聊天里的卡片和点开以后右侧栏里是同一个。
  // 队友说的话是别人的话：只当文字显示（Svelte 会转义）。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { LinesCardData } from "./interpreter";

  let { data }: PluginViewProps = $props();
  const card = $derived(data && typeof data === "object" && "lines" in data ? (data as LinesCardData) : null);
  const clock = (atMs: number) => new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
</script>

<div class="min-w-0" data-testid="interpreter-lines-card">
  {#if card}
    <p class="m-0 flex items-baseline gap-2 text-xs text-muted-foreground">
      <span class="min-w-0 flex-1 truncate" data-testid="interpreter-lines-summary">
        {card.total > card.lines.length
          ? m.plugin_interpreter_card_summary_more({ count: card.total, shown: card.lines.length, target: card.target })
          : m.plugin_interpreter_card_summary({ count: card.total, target: card.target })}
      </span>
    </p>
    <ul class="m-0 mt-1 list-none space-y-1 p-0">
      {#each card.lines as line, i (i)}
        <li class="min-w-0" data-testid="interpreter-line">
          <div class="flex items-baseline gap-2">
            <span class="min-w-0 flex-1 break-words text-[13px] text-foreground" data-testid="interpreter-line-text">{line.text}</span>
            <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{clock(line.atMs)}</span>
          </div>
          {#if line.heard}
            <p class="m-0 break-words text-[11px] text-muted-foreground" data-testid="interpreter-line-heard">
              {line.app ? m.plugin_interpreter_card_heard_in({ app: line.app, text: line.heard }) : m.plugin_interpreter_card_heard({ text: line.heard })}
            </p>
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_interpreter_card_empty()}</p>
  {/if}
</div>
