<script lang="ts">
  // 自动卡片（ADR 039 D3）：插件不叫她、自己发进聊天的一张卡片。上面一行写“{插件} 发来 · 时间”（宿主画，写明是哪个插件），
  // 下面是插件页面的卡片——和工具结果的卡片同一个组件（PluginCard：点开在右侧栏打开完整页面）。
  // 玩家点了右下角的消息提示（D4）：滚到这里，外圈闪一下主色。
  import { m } from "$lib/paraglide/messages.js";
  import type { AutoCardInfo } from "../agent";
  import { pluginText } from "../plugins";
  import PluginLogo from "../components/plugins/PluginLogo.svelte";
  import PluginCard from "./PluginCard.svelte";
  import { tryAgentChat } from "./context";

  let { card, atMs }: { card: AutoCardInfo; atMs: number } = $props();

  const chat = tryAgentChat();
  const name = $derived(pluginText(card.plugin, "name", card.pluginName));
  const time = $derived(new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }));
  const display = $derived({ structuredContent: card.structuredContent, ui: card.ui });

  let element = $state<HTMLElement>();
  let flashing = $state(false);
  let seen = 0;
  $effect(() => {
    const focus = chat?.focusCard;
    if (!focus || focus.id !== card.key || focus.n === seen || !element) return;
    seen = focus.n;
    element.scrollIntoView({ block: "center", behavior: "smooth" });
    flashing = true;
    const timer = setTimeout(() => (flashing = false), 1200);
    return () => clearTimeout(timer);
  });
</script>

<div bind:this={element} class="auto-card min-w-0 rounded-xl" class:flashing data-testid="auto-card" data-plugin={card.plugin}
  data-key={card.key} aria-label={m.chat_auto_card_label({ plugin: name, title: card.title })}>
  <p class="m-0 flex h-6 items-center gap-1.5 text-xs text-muted-foreground" data-testid="auto-card-from">
    <PluginLogo id={card.plugin} {name} size={16} />
    <span class="shrink-0 font-medium text-foreground/85">{m.chat_auto_card_from({ plugin: name })}</span>
    <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
    <span class="min-w-0 truncate">{card.title}</span>
    <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
    <span class="shrink-0 tabular-nums" aria-hidden="true">{time}</span>
  </p>
  <PluginCard {display} />
</div>

<style>
  .auto-card { transition: box-shadow 0.3s ease; }
  .auto-card.flashing { box-shadow: 0 0 0 2px var(--primary); }
  @media (prefers-reduced-motion: reduce) { .auto-card { transition: none; } }
</style>
