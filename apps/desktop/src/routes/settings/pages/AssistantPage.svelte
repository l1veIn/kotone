<script lang="ts">
  // 「琴音」页（ADR 014）：聊天（默认）与设置（AI 连接、热键、快捷指令、琴音 Agent）。
  // 顶栏参照 DeepSeek Harness 的会话头：页名 / 当前对话标题，右侧是进行中概况与分页切换。
  import { companionName } from "$lib/stores/companion";
  import { onMount, untrack } from "svelte";
  import { listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { useAgentChat } from "../../../lib/agentChat/context";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import AssistantChat from "./assistant/AssistantChat.svelte";
  import AssistantSettings from "./assistant/AssistantSettings.svelte";
  import AgentSettings from "./assistant/AgentSettings.svelte";
  import OpenRunsSummary from "./assistant/OpenRunsSummary.svelte";
  import IdentityTab from "./assistant/IdentityTab.svelte";
  import MemoryTab from "./assistant/MemoryTab.svelte";
  import SkillsTab from "./assistant/SkillsTab.svelte";
  import CompanionInputSettings from "./assistant/CompanionInputSettings.svelte";
  import WakeSettings from "./assistant/WakeSettings.svelte";
  import BubbleSettings from "./assistant/BubbleSettings.svelte";
  import OutputSettings from "./assistant/OutputSettings.svelte";
  import VoiceChatSettings from "./assistant/VoiceChatSettings.svelte";
  import UsageSettings from "./assistant/UsageSettings.svelte";

  type Section = "chat" | "identity" | "memory" | "skills" | "settings";
  let { onOpenConnections, onOpenAutomation, chatRequest = 0, open = null }: {
    onOpenConnections: () => void;
    onOpenAutomation: () => void;
    /** 侧边栏点了对话 / 新对话时递增：切回聊天分页。 */
    chatRequest?: number;
    /** 要打开某个分页（及「设置」里的某个分组）：首页入口、聊天里“去看看”的跳转。
     *  页面新挂载时也照它显示（从别的页跳过来收不到变化）。 */
    open?: { section: Section; anchor?: string; n: number } | null;
  } = $props();
  const chat = useAgentChat();
  let section = $state<Section>(untrack(() => open?.section ?? "chat"));
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  const ready = $derived(!!connections.find((item) => item.id === $settingsStore?.assistant.connectionId)?.hasApiKey);
  // ADR 023：聊天 · 角色（原“她是谁”）· 记忆 · 技能 · 设置
  const tabs = [
    { id: "chat", label: "聊天" }, { id: "identity", label: "角色" }, { id: "memory", label: "记忆" },
    { id: "skills", label: "技能" }, { id: "settings", label: "设置" },
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
    untrack(() => {
      section = request.section;
      if (request.anchor) scrollTo(request.anchor);
    });
  });
  function scrollTo(anchor: string) {
    // 分页刚切过来时还没渲染：等一帧再滚。
    requestAnimationFrame(() => document.getElementById(anchor)?.scrollIntoView({ block: "start" }));
  }
  function openSettings(anchor?: string) {
    section = "settings";
    if (anchor) scrollTo(anchor);
  }
  // 「设置」分页很长：顶部给几个跳转（原「交流」页的内容并进来了，ADR 023 修订）。
  const settingsLinks = [
    { id: "companion-settings-ai", label: "AI 连接" },
    { id: "companion-settings-call", label: "呼叫" },
    { id: "companion-settings-reply", label: "回答" },
    { id: "companion-settings-teammates", label: "游戏语音" },
    { id: "companion-settings-permissions", label: "权限" },
    { id: "companion-settings-usage", label: "用量" },
  ];

  onMount(() => {
    let disposed = false;
    void listConnections().then((items) => { if (!disposed) connections = items; })
      .catch((error) => { if (!disposed) connectionError = `读取连接失败：${errText(error)}`; });
    return () => { disposed = true; };
  });
</script>

{#snippet group(id: string, title: string, description: string)}
  <div id={id} class="scroll-mt-16 border-t border-hairline pt-6">
    <h2 class="m-0 text-[15px] font-semibold text-foreground">{title}</h2>
    <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{description}</p>
  </div>
{/snippet}

<div class="flex flex-col {section === 'chat' ? 'h-full' : ''}" data-testid="assistant-page">
  <!-- 分页切换放在页名旁（右上角留给提示消息） -->
  <header class="sticky top-0 z-20 flex h-13 shrink-0 items-center gap-3 border-b border-hairline bg-background/85 px-5 backdrop-blur">
    <h1 class="m-0 shrink-0 text-[15px] font-semibold text-foreground" data-testid="assistant-page-title">{name}</h1>
    <nav class="flex shrink-0 rounded-lg bg-muted/70 p-0.5" aria-label={`${name}分类`}>
      {#each tabs as tab}
        {@const active = section === tab.id}
        <button type="button" data-testid={`assistant-nav-${tab.id}`} aria-current={active ? "page" : undefined}
          class="h-7 rounded-md px-3 text-xs font-medium transition-colors {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => { section = tab.id; }}>{tab.label}</button>
      {/each}
    </nav>
    {#if section === "chat"}
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <span class="shrink-0 text-muted-foreground/50" aria-hidden="true">/</span>
        <p class="m-0 min-w-0 truncate text-[13px] text-muted-foreground" data-testid="agent-conversation-title">
          {chat.conversation?.title || "新对话"}
        </p>
        {#if chat.runningHere}<span class="flex shrink-0 items-center gap-1.5 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" role="status"><span class="size-1.5 animate-pulse rounded-full bg-primary"></span>回答中</span>{/if}
      </div>
      <div class="hidden min-w-0 lg:block"><OpenRunsSummary {onOpenAutomation} /></div>
    {/if}
  </header>
  {#if connectionError}<p role="alert" class="mx-5 mt-3 text-xs text-destructive">{connectionError}</p>{/if}
  <!-- 聊天一直挂着：切到设置再回来，草稿、流式输出和事件订阅都还在。 -->
  <div class="min-h-0 flex-1" class:hidden={section !== "chat"} data-testid="assistant-tasks">
    <AssistantChat {ready} active={section === "chat"} onOpenSettings={() => openSettings()} onOpenTalk={() => openSettings("companion-settings-call")} />
  </div>
  {#if section === "identity" || section === "memory" || section === "skills"}
    <div class="ui-scope mx-auto w-full max-w-3xl px-6 pt-7 pb-10 text-foreground" data-testid={`assistant-${section}-tab`}>
      {#if section === "identity"}<IdentityTab onOpenTalk={() => openSettings("companion-settings-call")} />
      {:else if section === "memory"}<MemoryTab />
      {:else}<SkillsTab />{/if}
    </div>
  {/if}
  {#if section === "settings"}
    <div class="ui-scope mx-auto w-full max-w-3xl space-y-6 px-6 pt-7 pb-10 text-foreground" data-testid="assistant-settings-tab">
      <div>
        <h2 class="m-0 text-xl font-semibold tracking-tight">{name}的设置</h2>
        <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">她用哪个 AI、怎么叫她、她怎么回答你、怎么替你跟队友说话，以及她能直接做哪些事。这些也都可以直接跟她说，让她帮你设。</p>
        <nav class="mt-3 flex flex-wrap gap-1.5" aria-label="设置分组" data-testid="companion-settings-links">
          {#each settingsLinks as link}
            <button type="button" class="h-7 rounded-md bg-muted/70 px-2.5 text-xs text-muted-foreground hover:text-foreground"
              onclick={() => openSettings(link.id)}>{link.label}</button>
          {/each}
        </nav>
        <div class="mt-3 lg:hidden"><OpenRunsSummary {onOpenAutomation} /></div>
      </div>
      <div id="companion-settings-ai" class="scroll-mt-16"><AssistantSettings {connections} {ready} {onOpenConnections} /></div>
      {@render group("companion-settings-call", "呼叫" + name, "按一下热键或喊她的名字，她就在右下角气泡里听你说。")}
      <CompanionInputSettings />
      <WakeSettings />
      {@render group("companion-settings-reply", "她怎么回答你", "右下角的对话气泡、语音和弹幕。")}
      <BubbleSettings />
      <OutputSettings />
      {@render group("companion-settings-teammates", "替你跟队友说话", "用虚拟麦克风替你在游戏语音里说话。")}
      <VoiceChatSettings />
      {@render group("companion-settings-permissions", "她能直接做什么", "游戏内交给她、每个工具允许 / 询问 / 禁止。")}
      <AgentSettings />
      <div id="companion-settings-usage" class="scroll-mt-16"><UsageSettings /></div>
    </div>
  {/if}
</div>
