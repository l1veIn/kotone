<script lang="ts">
  import { onMount } from "svelte";
  import { listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { errText, settingsStore } from "../../../lib/stores/ui";
  import ReminderTasks from "./assistant/ReminderTasks.svelte";
  import LegacyReminderSettings from "./assistant/LegacyReminderSettings.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import "./assistant/legacy-reminders.css";

  let { onOpenConnections }: { onOpenConnections: () => void } = $props();
  let section = $state<"tasks" | "settings">("tasks");
  let connections = $state<ConnectionInfo[]>([]);
  let connectionError = $state("");
  const ready = $derived(!!connections.find((item) => item.id === $settingsStore?.assistant.connectionId)?.hasApiKey);
  const tabs = [{ id: "tasks", label: "待办提醒" }, { id: "settings", label: "提醒设置" }] as const;

  onMount(() => {
    let disposed = false;
    void listConnections().then((items) => { if (!disposed) connections = items; })
      .catch((error) => { if (!disposed) connectionError = `读取连接失败：${errText(error)}`; });
    return () => { disposed = true; };
  });
</script>

<SettingsPage title="提醒助手" testid="legacy-reminders-page">
<div class="legacy-reminders">
  <nav class="mt-4 flex flex-wrap gap-1.5" aria-label="提醒助手分类">
    {#each tabs as tab}
      {@const active = section === tab.id}
      <button type="button" data-testid={`assistant-nav-${tab.id}`} aria-current={active ? "page" : undefined}
        class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition
          {active ? 'bg-kotone-cyan/15 text-primary ring-kotone-cyan/40' : 'bg-white/5 text-muted-foreground ring-white/10 hover:bg-white/10 hover:text-foreground'}"
        onclick={() => { section = tab.id; }}>{tab.label}</button>
    {/each}
  </nav>
  <div class="mt-4">
    {#if connectionError}<p role="alert" class="mb-3 text-xs text-destructive">{connectionError}</p>{/if}
    <!-- Keep drafts, in-flight requests and reminder subscriptions alive across sections. -->
    <div class:hidden={section !== "tasks"} data-testid="assistant-tasks">
      <ReminderTasks {ready} active={section === "tasks"} onOpenSettings={() => { section = "settings"; }} />
    </div>
    {#if section === "settings"}
      <LegacyReminderSettings {connections} {ready} {onOpenConnections} />
    {/if}
  </div>
</div>
</SettingsPage>
