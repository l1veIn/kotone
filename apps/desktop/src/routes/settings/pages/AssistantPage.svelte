<script lang="ts">
  // 角色页（ADR 014 / 028）：聊天（主界面，从侧栏的新对话 / 对话列表进入）与角色本身——角色 · 记忆 · 技能。
  // 原来的「设置」分页整页搬进了「设置」的陪玩组（D11）：换角色时这些设置共用，不属于某个角色。
  // 顶栏参照 DeepSeek Harness 的会话头：页名 / 当前对话标题，右侧是进行中概况。
  import { companionName } from "$lib/stores/companion";
  import { onMount, untrack } from "svelte";
  import { companionBindings, listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { usesLegacyVoiceInput } from "../../../lib/ipc/contracts";
  import { useAgentChat } from "../../../lib/agentChat/context";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import type { AssistantSection } from "../../../lib/stores/navigation";
  import type { DeepSeekSetupResult } from "../../../lib/deepseekSetup";
  import AssistantChat from "./assistant/AssistantChat.svelte";
  import OpenRunsSummary from "./assistant/OpenRunsSummary.svelte";
  import IdentityTab from "./assistant/IdentityTab.svelte";
  import MemoryTab from "./assistant/MemoryTab.svelte";
  import SkillsTab from "./assistant/SkillsTab.svelte";

  let { onOpenConnections, onOpenAutomation, onOpenSettings, onOpenTalk, onKeyConnected, onBackToVoice, chatRequest = 0, open = null }: {
    onOpenConnections: () => void;
    onOpenAutomation: () => void;
    /** 「设置」的陪玩组。 */
    onOpenSettings: () => void;
    /** 「设置 → 怎么叫她」。 */
    onOpenTalk: () => void;
    onKeyConnected: (result: DeepSeekSetupResult) => void;
    onBackToVoice: () => void;
    /** 侧边栏点了对话 / 新对话时递增：切回聊天。 */
    chatRequest?: number;
    /** 要打开某个分页：侧栏的角色入口、聊天里“去看看”的跳转。页面新挂载时也照它显示。 */
    open?: { section: AssistantSection; n: number } | null;
  } = $props();
  const chat = useAgentChat();
  let section = $state<AssistantSection>(untrack(() => open?.section ?? "chat"));
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  const legacy = $derived(usesLegacyVoiceInput($settingsStore?.ui));
  const ready = $derived(!legacy && !!connections.find((item) => item.id === companionBindings($settingsStore?.assistant)?.connectionId)?.hasApiKey);
  // ADR 028：角色页只留 角色（原“她是谁”）· 记忆 · 技能
  const tabs = [
    { id: "identity", label: "角色" }, { id: "memory", label: "记忆" }, { id: "skills", label: "技能" },
  ] as const;
  const name = $derived($companionName);

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
    untrack(() => { section = request.section; });
  });

  function refreshConnections() {
    void listConnections().then((items) => { connections = items; connectionError = ""; })
      .catch((error) => { connectionError = `读取连接失败：${errText(error)}`; });
  }
  onMount(refreshConnections);
  // Key 卡片、设置页、琴音自己都可能改连接：跟着设置刷新。
  $effect(() => {
    $settingsStore?.connections;
    $settingsStore?.assistant;
    untrack(refreshConnections);
  });
</script>

<div class="flex flex-col {section === 'chat' ? 'h-full' : ''}" data-testid="assistant-page">
  <header class="sticky top-0 z-20 flex h-13 shrink-0 items-center gap-3 border-b border-hairline bg-background/85 px-5 backdrop-blur">
    <h1 class="m-0 shrink-0 text-[15px] font-semibold text-foreground" data-testid="assistant-page-title">{name}</h1>
    {#if section === "chat"}
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <span class="shrink-0 text-muted-foreground/50" aria-hidden="true">/</span>
        <p class="m-0 min-w-0 truncate text-[13px] text-muted-foreground" data-testid="agent-conversation-title">
          {chat.conversation?.title || "新对话"}
        </p>
        {#if chat.runningHere}<span class="flex shrink-0 items-center gap-1.5 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" role="status"><span class="size-1.5 animate-pulse rounded-full bg-primary"></span>回答中</span>{/if}
      </div>
      <div class="hidden min-w-0 lg:block"><OpenRunsSummary {onOpenAutomation} /></div>
    {:else}
      <nav class="flex shrink-0 rounded-lg bg-muted/70 p-0.5" aria-label={`${name}分类`}>
        {#each tabs as tab}
          {@const active = section === tab.id}
          <button type="button" data-testid={`assistant-nav-${tab.id}`} aria-current={active ? "page" : undefined}
            class="h-7 rounded-md px-3 text-xs font-medium transition-colors {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            onclick={() => { section = tab.id; }}>{tab.label}</button>
        {/each}
      </nav>
      <div class="flex-1"></div>
      <button type="button" class="shrink-0 text-xs text-muted-foreground hover:text-foreground" data-testid="assistant-open-settings" onclick={onOpenSettings}>{name}的设置在「设置」里 →</button>
    {/if}
  </header>
  {#if connectionError}<p role="alert" class="mx-5 mt-3 text-xs text-destructive">{connectionError}</p>{/if}
  <!-- 聊天一直挂着：切到角色分页再回来，草稿、流式输出和事件订阅都还在。 -->
  <div class="min-h-0 flex-1" class:hidden={section !== "chat"} data-testid="assistant-tasks">
    <AssistantChat {ready} {legacy} active={section === "chat"} {onOpenSettings} {onOpenTalk} {onOpenConnections} {onKeyConnected} {onBackToVoice} />
  </div>
  {#if section !== "chat"}
    <div class="ui-scope mx-auto w-full max-w-3xl px-6 pt-7 pb-10 text-foreground" data-testid={`assistant-${section}-tab`}>
      {#if section === "identity"}<IdentityTab {onOpenTalk} />
      {:else if section === "memory"}<MemoryTab />
      {:else}<SkillsTab />{/if}
    </div>
  {/if}
</div>
