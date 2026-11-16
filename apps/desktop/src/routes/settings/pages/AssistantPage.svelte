<script lang="ts">
  // 「琴音」页（ADR 014）：聊天（默认）与设置（AI 连接、能力自检、热键、快捷指令、琴音 Agent）。
  // 顶栏参照 DeepSeek Harness 的会话头：页名 / 当前对话标题，右侧是进行中概况与分页切换。
  import { onMount } from "svelte";
  import { listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { useAgentChat } from "../../../lib/agentChat/context";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import AssistantChat from "./assistant/AssistantChat.svelte";
  import AssistantSettings from "./assistant/AssistantSettings.svelte";
  import AgentSettings from "./assistant/AgentSettings.svelte";
  import OpenRunsSummary from "./assistant/OpenRunsSummary.svelte";

  let { onOpenConnections, onOpenAutomation, chatRequest = 0 }: {
    onOpenConnections: () => void;
    onOpenAutomation: () => void;
    /** 侧边栏点了对话 / 新对话时递增：切回聊天分页。 */
    chatRequest?: number;
  } = $props();
  const chat = useAgentChat();
  let section = $state<"chat" | "settings">("chat");
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  const ready = $derived(!!connections.find((item) => item.id === $settingsStore?.assistant.connectionId)?.hasApiKey);
  const tabs = [{ id: "chat", label: "聊天" }, { id: "settings", label: "设置" }] as const;

  // 只在请求号变化时切换（首次挂载不算）。
  let seenRequest: number | null = null;
  $effect(() => {
    const request = chatRequest;
    if (seenRequest !== null && request !== seenRequest) section = "chat";
    seenRequest = request;
  });

  onMount(() => {
    let disposed = false;
    void listConnections().then((items) => { if (!disposed) connections = items; })
      .catch((error) => { if (!disposed) connectionError = `读取连接失败：${errText(error)}`; });
    return () => { disposed = true; };
  });
</script>

<div class="flex flex-col {section === 'chat' ? 'h-full' : ''}" data-testid="assistant-page">
  <!-- 分页切换放在页名旁（右上角留给提示消息） -->
  <header class="sticky top-0 z-20 flex h-13 shrink-0 items-center gap-3 border-b border-hairline bg-background/85 px-5 backdrop-blur">
    <h1 class="m-0 shrink-0 text-[15px] font-semibold text-foreground">琴音</h1>
    <nav class="flex shrink-0 rounded-lg bg-muted/70 p-0.5" aria-label="琴音分类">
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
    <AssistantChat {ready} active={section === "chat"} onOpenSettings={() => { section = "settings"; }} />
  </div>
  {#if section === "settings"}
    <div class="ui-scope mx-auto w-full max-w-3xl space-y-6 px-6 pt-7 pb-10 text-foreground" data-testid="assistant-settings-tab">
      <div>
        <h2 class="m-0 text-xl font-semibold tracking-tight">琴音设置</h2>
        <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">AI 连接、能力自检、热键与快捷指令，以及琴音能直接做哪些事。</p>
        <div class="mt-3 lg:hidden"><OpenRunsSummary {onOpenAutomation} /></div>
      </div>
      <AssistantSettings {connections} {ready} {onOpenConnections} />
      <AgentSettings />
    </div>
  {/if}
</div>
