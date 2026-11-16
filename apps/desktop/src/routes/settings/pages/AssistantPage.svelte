<script lang="ts">
  // 「琴音」页（ADR 014）：聊天（默认）与设置（AI 连接、能力自检、热键、快捷指令、琴音 Agent）。
  import { onMount } from "svelte";
  import { listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import AssistantChat from "./assistant/AssistantChat.svelte";
  import AssistantSettings from "./assistant/AssistantSettings.svelte";
  import AgentSettings from "./assistant/AgentSettings.svelte";
  import "./panel.css";

  let { onOpenConnections, onOpenAutomation }: { onOpenConnections: () => void; onOpenAutomation: () => void } = $props();
  let section = $state<"chat" | "settings">("chat");
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  const ready = $derived(!!connections.find((item) => item.id === $settingsStore?.assistant.connectionId)?.hasApiKey);
  const tabs = [{ id: "chat", label: "聊天" }, { id: "settings", label: "设置" }] as const;

  onMount(() => {
    let disposed = false;
    void listConnections().then((items) => { if (!disposed) connections = items; })
      .catch((error) => { if (!disposed) connectionError = `读取连接失败：${errText(error)}`; });
    return () => { disposed = true; };
  });
</script>

<div class="panel-page flex flex-col px-6 py-5 {section === 'chat' ? 'h-[calc(100%-1rem)] min-h-[32rem]' : ''}" data-testid="assistant-page">
  <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
    <h1 class="text-lg font-bold">琴音</h1>
    <nav class="flex gap-1.5" aria-label="琴音分类">
      {#each tabs as tab}
        {@const active = section === tab.id}
        <button type="button" data-testid={`assistant-nav-${tab.id}`} aria-current={active ? "page" : undefined}
          class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition
            {active ? 'bg-kotone-cyan/15 text-kotone-cyan ring-kotone-cyan/40' : 'bg-white/5 text-white/55 ring-white/10 hover:bg-white/10 hover:text-white/80'}"
          onclick={() => { section = tab.id; }}>{tab.label}</button>
      {/each}
    </nav>
  </div>
  {#if connectionError}<p role="alert" class="mt-3 text-xs text-kotone-pink">{connectionError}</p>{/if}
  <!-- 聊天一直挂着：切到设置再回来，草稿、流式输出和事件订阅都还在。 -->
  <div class="mt-4 min-h-0 flex-1" class:hidden={section !== "chat"} data-testid="assistant-tasks">
    <AssistantChat {ready} active={section === "chat"} onOpenSettings={() => { section = "settings"; }} {onOpenAutomation} />
  </div>
  {#if section === "settings"}
    <div class="mt-4">
      <AssistantSettings {connections} {ready} {onOpenConnections} />
      <AgentSettings />
    </div>
  {/if}
</div>
