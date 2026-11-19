<script lang="ts" module>
  export type PageId =
    | "general" | "processing" | "assistant" | "automation" | "game"
    | "history" | "advanced" | "about" | "character" | "legacy-reminders";
</script>

<script lang="ts">
  /*
   * 设置窗口侧边栏（参照 DeepSeek Harness 的侧边栏：顶部“新对话”、功能导航、会话列表、
   * 底部固定的设置项）。三种形态：展开（236px）、图标栏（56px）、窄窗口时的抽屉。
   * Kotone 的气质：当前项的青色指示条与图标高亮，底部的琴音形象卡。
   */
  import { companionName } from "$lib/stores/companion";
  import {
    Bell, Bot, Gamepad2, History, Info, MessageSquarePlus, Mic, Settings2, Sparkles, Zap,
  } from "@lucide/svelte";
  import type { Component } from "svelte";
  import ConversationList from "../../lib/agentChat/ConversationList.svelte";
  import { useAgentChat } from "../../lib/agentChat/context";
  import { toast, errText, settingsStore } from "../../lib/stores/ui";
  import { hasLegacyReminders } from "../../lib/ipc";
  import iconSrc from "../../assets/brand/icon-src.webp";

  let { page, rail = false, legacyVoiceOnly = false, onNavigate, onOpenChat }: {
    page: PageId;
    legacyVoiceOnly?: boolean;
    /** 收成图标栏。 */
    rail?: boolean;
    onNavigate: (id: PageId) => void;
    /** 打开「琴音」聊天（新对话、点了某段对话之后）。 */
    onOpenChat: () => void;
  } = $props();

  const chat = useAgentChat();
  // ADR 023：侧栏分两组。语音输入仍排第一（首页），游戏陪玩是第二组；页面内部标识不变。
  // 「琴音」一项用她现在的名字（ADR 022：可以改名）。
  const groups: { label: string; items: { id: PageId; label: string; icon: Component }[] }[] = $derived([
    {
      label: "语音输入",
      items: [
        { id: "general", label: "语音输入", icon: Mic },
        { id: "processing", label: "文字处理", icon: Sparkles },
        { id: "history", label: "历史记录", icon: History },
        ...(legacyVoiceOnly ? [{ id: "game" as const, label: "游戏适配", icon: Gamepad2 }] : []),
        ...(hasLegacyReminders($settingsStore?.ui) ? [{ id: "legacy-reminders" as const, label: "提醒助手", icon: Bell }] : []),
      ],
    },
    ...(!legacyVoiceOnly ? [{
      label: "游戏陪玩",
      items: [
        { id: "assistant" as const, label: $companionName, icon: Bot },
        { id: "game" as const, label: "游戏列表", icon: Gamepad2 },
        { id: "automation" as const, label: "自动化", icon: Zap },
      ],
    }] : []),
  ]);
  const footItems: { id: PageId; label: string; icon: Component }[] = [
    { id: "advanced", label: "设置", icon: Settings2 },
    { id: "about", label: "关于", icon: Info },
  ];

  async function guarded(action: () => Promise<void>, failure: string) {
    try { await action(); onOpenChat(); }
    catch (error) { toast(false, `${failure}：${errText(error)}`); }
  }
  function newChat() {
    if (chat.currentId === null || chat.running) { onOpenChat(); return; }
    void guarded(() => chat.newConversation(), "新建对话失败");
  }
</script>

{#snippet navItem(item: { id: PageId; label: string; icon: Component })}
  {@const active = page === item.id || (page === "character" && item.id === "about")}
  {@const Icon = item.icon}
  <button
    type="button"
    data-testid={`settings-nav-${item.id}`}
    aria-current={active ? "page" : undefined}
    aria-label={rail ? item.label : undefined}
    title={rail ? item.label : undefined}
    class="group relative flex h-8 w-full items-center gap-2.5 rounded-lg text-left text-[13px] transition-colors
      {rail ? 'justify-center' : 'px-2.5'}
      {active ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground' : 'text-sidebar-foreground hover:bg-sidebar-accent/60 hover:text-foreground'}"
    onclick={() => onNavigate(item.id)}
  >
    {#if active}<span class="absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-full bg-primary shadow-[0_0_8px_var(--color-primary)]"></span>{/if}
    <Icon class="size-4 shrink-0 {active ? 'text-primary' : 'text-sidebar-foreground/75 group-hover:text-foreground'}" />
    {#if !rail}<span class="truncate">{item.label}</span>{/if}
  </button>
{/snippet}

<aside
  class="flex h-full shrink-0 flex-col bg-sidebar pt-1 pb-2 text-sidebar-foreground {rail ? 'w-14 px-2' : 'w-59 px-2'}"
  data-testid="app-sidebar" data-rail={rail}
>
  {#if !legacyVoiceOnly}
  <button type="button" data-testid="sidebar-new-chat" onclick={newChat}
    aria-label={rail ? "新对话" : undefined} title={rail ? "新对话" : undefined}
    class="flex h-9 w-full shrink-0 items-center gap-2.5 rounded-lg border border-hairline text-[13px] text-foreground transition-colors hover:border-primary/40 hover:bg-sidebar-accent/60
      {rail ? 'justify-center' : 'px-2.5'}">
    <MessageSquarePlus class="size-4 shrink-0 text-primary" />
    {#if !rail}<span>新对话</span><span class="ml-auto text-[11px] text-sidebar-foreground/50">{$companionName}</span>{/if}
  </button>

  {/if}
  <nav class="mt-3 flex shrink-0 flex-col gap-px" aria-label="设置分类">
    {#each groups as group, index (group.label)}
      {#if rail}
        {#if index > 0}<div class="mx-2 my-1.5 border-t border-hairline"></div>{/if}
      {:else}
        <p class="m-0 px-2.5 pb-1 {index > 0 ? 'pt-3' : ''} text-[11px] font-medium text-sidebar-foreground/50" data-testid={`sidebar-group-${index}`}>{group.label}</p>
      {/if}
      {#each group.items as item (item.id)}{@render navItem(item)}{/each}
    {/each}
  </nav>

  {#if !rail && !legacyVoiceOnly}
    <div class="mt-4 flex min-h-0 flex-1 flex-col">
      <ConversationList
        conversations={chat.state?.conversations ?? []}
        currentId={chat.currentId}
        runningId={chat.runningId}
        activePage={page === "assistant"}
        onNew={() => void guarded(() => chat.newConversation(), "新建对话失败")}
        onSelect={(id) => {
          if (id === chat.currentId) { onOpenChat(); return; }
          void guarded(() => chat.select(id), "切换对话失败");
        }}
        onDelete={(id) => chat.remove(id).catch((error) => { toast(false, `删除对话失败：${errText(error)}`); })}
      />
    </div>
  {:else}
    <div class="flex-1"></div>
  {/if}

  <div class="flex shrink-0 flex-col gap-px border-t border-hairline pt-2">
    {#each footItems as item (item.id)}{@render navItem(item)}{/each}
    {#if !rail}
      <div class="mt-2 flex items-center gap-2 px-2.5 text-[11px] text-sidebar-foreground/45">
        <img src={iconSrc} alt="" class="size-4 rounded object-cover opacity-80" />
        <span class="truncate">反馈群：1092354484</span>
      </div>
    {/if}
  </div>
</aside>
