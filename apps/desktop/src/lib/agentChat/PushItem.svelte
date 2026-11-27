<script lang="ts">
  // 推送（ADR 043 D3；以前叫自动卡片，ADR 039 D3）：插件不叫她、自己发进聊天的一条。上面一行写“{插件} 推送 · 名字 · 时间”
  // （宿主画，写明是哪个插件发的）；下面照样子画：一行（几格，带好坏的颜色）、语音（她说的那句）、卡片（插件页面的卡片——
  // 和工具结果的卡片同一个组件 PluginCard：点开在右侧栏打开完整页面）。同一个 key 再推一次时这一条原地重画，位置不动
  // （卡片里的 MCP App 收到新的数据、不重新加载）；带提醒的更新挪到聊天最下面（重新画一张，ADR 043 D5 修订）。
  // 旧对话里的自动卡片也画成这个（只有卡片）。
  // 玩家点了悬浮窗里的卡片（ADR 043 D6）：滚到这里，外圈闪一下主色。悬浮窗的小窗里也画它：给了 `onOpen`，
  // 卡片上面那一行就交给它（打开 Kotone、定位到这一条），不在右侧栏打开。
  import { Volume2 } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import type { PushRecord, PushTone } from "../agent";
  import { pushFromName, pushTitle } from "../plugins";
  import PluginLogo from "../components/plugins/PluginLogo.svelte";
  import PluginCard from "./PluginCard.svelte";
  import { tryAgentChat } from "./context";

  let { push, atMs, onOpen }: { push: PushRecord; atMs: number; onOpen?: () => void } = $props();

  const chat = tryAgentChat();
  const name = $derived(pushFromName(push.plugin, push.pluginName));
  /** 有卡片用卡片的名字（插件给的），否则用这条推送在清单里的名字（插件的文案）。 */
  const title = $derived(push.card?.title ?? (push.trigger ? pushTitle(push.plugin, { id: push.trigger, title: push.title }) : push.title));
  const time = $derived(new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }));
  const display = $derived(push.card ? { structuredContent: push.card.structuredContent, ui: push.card.ui } : null);
  const said = $derived(push.said ?? push.voice?.[0] ?? "");
  const TONES: Record<PushTone, string> = {
    good: "bg-success/12 text-success",
    ok: "bg-info/12 text-info",
    bad: "bg-warning/14 text-warning",
  };

  let element = $state<HTMLElement>();
  let flashing = $state(false);
  let seen = 0;
  $effect(() => {
    const focus = chat?.focusCard;
    if (!focus || focus.id !== push.key || focus.n === seen || !element) return;
    seen = focus.n;
    element.scrollIntoView({ block: "center", behavior: "smooth" });
    flashing = true;
    const timer = setTimeout(() => (flashing = false), 1200);
    return () => clearTimeout(timer);
  });
</script>

<div bind:this={element} class="push-item min-w-0 rounded-xl" class:flashing data-testid="push-item" data-plugin={push.plugin}
  data-key={push.key} data-trigger={push.trigger} aria-label={m.chat_push_label({ plugin: name, title })}>
  <p class="m-0 flex h-6 items-center gap-1.5 text-xs text-muted-foreground" data-testid="push-from">
    <PluginLogo id={push.plugin} {name} size={16} />
    <span class="shrink-0 font-medium text-foreground/85">{m.chat_push_from({ plugin: name })}</span>
    <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
    <span class="min-w-0 truncate" data-testid="push-title">{title}</span>
    <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
    <span class="shrink-0 tabular-nums" aria-hidden="true">{time}</span>
  </p>
  {#if push.line?.cells.length}
    <!-- 一行：几格，各带好坏（悬浮窗气泡里显示的就是这一行） -->
    <div class="mt-1 flex max-w-[34rem] flex-wrap gap-1.5" data-testid="push-line">
      {#each push.line.cells as cell, i (i)}
        <span class="rounded-md px-2 py-0.5 text-[13px] font-medium {cell.tone ? TONES[cell.tone] : 'bg-muted/60 text-foreground/85'}"
          data-testid="push-cell" data-tone={cell.tone ?? ""}>{cell.text}</span>
      {/each}
    </div>
  {/if}
  {#if said}
    <!-- 语音：她说的那句固定的话（不调模型） -->
    <p class="m-0 mt-1 flex items-center gap-1.5 text-[13px] text-foreground/85" data-testid="push-voice">
      <Volume2 class="size-3.5 shrink-0 text-muted-foreground" aria-label={m.chat_push_voice()} />
      <span class="min-w-0">{said}</span>
    </p>
  {/if}
  {#if display}
    <PluginCard {display} {onOpen} />
  {/if}
</div>

<style>
  .push-item { transition: box-shadow 0.3s ease; }
  .push-item.flashing { box-shadow: 0 0 0 2px var(--primary); }
  @media (prefers-reduced-motion: reduce) { .push-item { transition: none; } }
</style>
