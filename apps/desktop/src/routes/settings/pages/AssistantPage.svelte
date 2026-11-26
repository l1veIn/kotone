<script lang="ts">
  // 角色页（ADR 014 / 028）：聊天（主界面，从侧栏的新对话 / 对话列表进入），和角色的分页
  // 角色 · 记忆 · 技能 · 唤醒 · 交互 · 权限 · 用量。后四个原来是「设置」的陪玩组（怎么叫她 · 她怎么回答 · 权限 · 用量），
  // 玩家 2026-11-25 说“陪玩相关的设置项都应该在游戏陪玩 → 角色页面配置”，整排搬过来，内容不变（交接第十一节第 8 条）。
  // 这些设置换角色时共用，不属于某个角色；选她用的模型挪回了聊天页输入框旁边。
  // 顶栏参照 DeepSeek Harness 的会话头：页名 / 当前对话标题，右侧是进行中概况。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
  import { onMount, tick, untrack } from "svelte";
  import { companionBindings, listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { usesLegacyVoiceInput } from "../../../lib/ipc/contracts";
  import { useAgentChat } from "../../../lib/agentChat/context";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import { CHAT_MODEL_ANCHOR, isCompanionSection, type AssistantSection } from "../../../lib/stores/navigation";
  import type { DeepSeekSetupResult } from "../../../lib/deepseekSetup";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import AssistantChat from "./assistant/AssistantChat.svelte";
  import OpenRunsSummary from "./assistant/OpenRunsSummary.svelte";
  import IdentityTab from "./assistant/IdentityTab.svelte";
  import MemoryTab from "./assistant/MemoryTab.svelte";
  import SkillsTab from "./assistant/SkillsTab.svelte";
  import CompanionInputSettings from "./assistant/CompanionInputSettings.svelte";
  import WakeSettings from "./assistant/WakeSettings.svelte";
  import BubbleSettings from "./assistant/BubbleSettings.svelte";
  import OutputSettings from "./assistant/OutputSettings.svelte";
  import AgentSettings from "./assistant/AgentSettings.svelte";
  import UsageSettings from "./assistant/UsageSettings.svelte";
  import { PanelRight } from "@lucide/svelte";
  import { rightSidebarOpen, toggleRightSidebar } from "../../../lib/stores/rightSidebar";

  let { onOpenConnections, onOpenAutomation, onKeyConnected, onBackToVoice, chatRequest = 0, open = null }: {
    onOpenConnections: () => void;
    onOpenAutomation: () => void;
    onKeyConnected: (result: DeepSeekSetupResult) => void;
    onBackToVoice: () => void;
    /** 侧边栏点了对话 / 新对话时递增：切回聊天。 */
    chatRequest?: number;
    /** 要打开某个分页（和分页里的分组）：侧栏的角色入口、聊天里“去看看”的跳转。页面新挂载时也照它显示。 */
    open?: { section: AssistantSection; anchor?: string; n: number } | null;
  } = $props();
  const chat = useAgentChat();
  let section = $state<AssistantSection>(untrack(() => open?.section ?? "chat"));
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  let pageEl = $state<HTMLElement>();
  /** 每加一，聊天页就打开一次输入框旁的模型菜单（“去看看：她用的模型”）。 */
  let modelRequest = $state(0);
  const legacy = $derived(usesLegacyVoiceInput($settingsStore?.ui));
  const ready = $derived(!legacy && !!connections.find((item) => item.id === companionBindings($settingsStore?.assistant)?.connectionId)?.hasApiKey);
  // 角色 · 记忆 · 技能；唤醒 · 交互 · 权限 · 用量（逃生通道用户还没启用她，没有后四个，和原来设置里不显示陪玩组一样）
  const baseTabs = [
    { id: "identity", label: m.companion_page_tab_identity() }, { id: "memory", label: m.companion_page_tab_memory() }, { id: "skills", label: m.companion_page_tab_skills() },
  ] as const;
  const companionTabs = [
    { id: "call", label: m.companion_page_tab_call() }, { id: "reply", label: m.companion_page_tab_reply() },
    { id: "permissions", label: m.companion_page_tab_permissions() }, { id: "usage", label: m.companion_page_tab_usage() },
  ] as const;
  const tabs = $derived<readonly { id: AssistantSection; label: string }[]>(legacy ? baseTabs : [...baseTabs, ...companionTabs]);
  const name = $derived($companionName);

  /** 切到一个分页；有分组就滚到分组，没有就回到顶上。 */
  function show(next: AssistantSection, anchor?: string) {
    // 逃生通道用户没有唤醒 / 交互 / 权限 / 用量：落到聊天页的 Key 卡片（先启用她）
    const target = legacy && isCompanionSection(next) ? "chat" : next;
    section = target;
    if (target === "chat") {
      if (anchor === CHAT_MODEL_ANCHOR) modelRequest++;
      return;
    }
    void tick().then(() => requestAnimationFrame(() => {
      const el = anchor ? document.getElementById(anchor) : null;
      if (el) el.scrollIntoView({ block: "start" });
      else pageEl?.closest("main")?.scrollTo({ top: 0 });
    }));
  }

  // 只在请求号变化时切换（首次挂载不算）。
  let seenRequest: number | null = null;
  $effect(() => {
    const request = chatRequest;
    if (seenRequest !== null && request !== seenRequest) section = "chat";
    seenRequest = request;
  });
  $effect(() => {
    const request = open;
    if (!request) return;
    untrack(() => show(request.section, request.anchor));
  });

  function refreshConnections() {
    void listConnections().then((items) => { connections = items; connectionError = ""; })
      .catch((error) => { connectionError = m.companion_page_connections_load_failed({ error: errText(error) }); });
  }
  onMount(refreshConnections);
  // Key 卡片、设置页、聊天页的模型菜单、琴音自己都可能改连接：跟着设置刷新。
  $effect(() => {
    $settingsStore?.connections;
    $settingsStore?.assistant;
    untrack(refreshConnections);
  });
</script>

<div class="flex flex-col {section === 'chat' ? 'h-full' : ''}" data-testid="assistant-page" bind:this={pageEl}>
  <header class="sticky top-0 z-20 flex h-13 shrink-0 items-center gap-3 border-b border-hairline bg-background/85 px-5 backdrop-blur">
    <h1 class="m-0 shrink-0 text-[15px] font-semibold text-foreground" data-testid="assistant-page-title">{name}</h1>
    {#if section === "chat"}
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <span class="shrink-0 text-muted-foreground/50" aria-hidden="true">/</span>
        <p class="m-0 min-w-0 truncate text-[13px] text-muted-foreground" data-testid="agent-conversation-title">
          {chat.conversation?.title || m.companion_page_new_conversation()}
        </p>
        {#if chat.runningHere}<span class="flex shrink-0 items-center gap-1.5 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" role="status"><span class="size-1.5 animate-pulse rounded-full bg-primary"></span>{m.companion_page_answering()}</span>{/if}
      </div>
      <div class="hidden min-w-0 lg:block"><OpenRunsSummary {onOpenAutomation} /></div>
      <!-- 右侧栏开关（ADR 031：网页与后台在做的事） -->
      <button type="button" data-testid="right-sidebar-toggle" aria-pressed={$rightSidebarOpen}
        aria-label={$rightSidebarOpen ? m.companion_page_right_sidebar_hide() : m.companion_page_right_sidebar_show_label()} title={$rightSidebarOpen ? m.companion_page_right_sidebar_hide() : m.companion_page_right_sidebar_show_title()}
        class="flex size-7 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-muted {$rightSidebarOpen ? 'text-foreground' : 'text-muted-foreground'}"
        onclick={toggleRightSidebar}><PanelRight class="size-4" /></button>
    {:else}
      <!-- 七个分页一排；窗口窄时横着滚，不换行（顶栏高度固定） -->
      <nav class="flex min-w-0 overflow-x-auto rounded-lg bg-muted/70 p-0.5 [scrollbar-width:none]" aria-label={m.companion_page_tabs_label({ name })} data-testid="assistant-nav">
        {#each tabs as tab (tab.id)}
          {@const active = section === tab.id}
          <button type="button" data-testid={`assistant-nav-${tab.id}`} aria-current={active ? "page" : undefined}
            class="h-7 shrink-0 rounded-md px-3 text-xs font-medium whitespace-nowrap transition-colors {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            onclick={() => show(tab.id)}>{tab.label}</button>
        {/each}
      </nav>
    {/if}
  </header>
  {#if connectionError}<p role="alert" class="mx-5 mt-3 text-xs text-destructive">{connectionError}</p>{/if}
  <!-- 聊天一直挂着：切到角色分页再回来，草稿、流式输出和事件订阅都还在。 -->
  <div class="min-h-0 flex-1" class:hidden={section !== "chat"} data-testid="assistant-tasks">
    <AssistantChat {ready} {legacy} {connections} {modelRequest} active={section === "chat"}
      onOpenTalk={() => show("call", "settings-call-hotkey")} {onOpenConnections} {onKeyConnected} {onBackToVoice} />
  </div>
  {#if section !== "chat"}
    <div class="ui-scope mx-auto w-full max-w-3xl px-6 pt-7 pb-10 text-foreground" data-testid={`assistant-${section}-tab`}>
      {#if section === "identity"}<IdentityTab onOpenTalk={() => show("call", "settings-call-wake")} />
      {:else if section === "memory"}<MemoryTab />
      {:else if section === "skills"}<SkillsTab />
      {:else if section === "call"}
        <!-- 唤醒（原「设置 → 怎么叫她」）：呼叫键、通话、快捷指令；语音唤醒；聊天页的呼叫按钮 -->
        <div class="space-y-6" data-testid="settings-call">
          <div id="settings-call-hotkey" class="scroll-mt-16"><CompanionInputSettings /></div>
          <div id="settings-call-wake" class="scroll-mt-16"><WakeSettings /></div>
          <SettingsSection title={m.system_advanced_call_button_title()} description={m.system_advanced_call_button_description({ name })}>
            <p class="m-0 py-3 text-xs text-muted-foreground">{m.system_advanced_call_button_hint()}</p>
          </SettingsSection>
        </div>
      {:else if section === "reply"}
        <!-- 交互（原「设置 → 她怎么回答」）：悬浮窗（她的气泡、推送的一行和小窗，ADR 043 D6）、声音与弹幕。
             她用的模型不在这里了：在聊天页输入框旁边，这里留一行指过去。 -->
        <div class="space-y-6" data-testid="settings-reply">
          <p class="mt-0 text-xs text-muted-foreground" data-testid="settings-reply-model-moved">
            {m.companion_page_model_moved()}<button type="button" class="ml-1 font-medium text-primary hover:underline" data-testid="settings-reply-open-model"
              onclick={() => show("chat", CHAT_MODEL_ANCHOR)}>{m.companion_page_model_open()}</button>
          </p>
          <div id="settings-reply-bubble" class="scroll-mt-16"><BubbleSettings /></div>
          <div id="settings-reply-output" class="scroll-mt-16"><OutputSettings /></div>
        </div>
      {:else if section === "permissions"}
        <div class="space-y-6" data-testid="settings-permissions"><AgentSettings /></div>
      {:else}
        <div data-testid="settings-usage"><UsageSettings /></div>
      {/if}
    </div>
  {/if}
</div>
