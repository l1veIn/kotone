<script lang="ts">
  // 「自动化」页（ADR 013 §8）：进行中（等待中的提醒与自动化、运行中的自动化）/ 已保存 / 记录。
  import { onMount } from "svelte";
  import {
    listAutomations, listRuns, stopRuns, subscribeAutomations,
    type Automation, type AutomationRun, type RunsSnapshot,
  } from "../../../lib/automation";
  import { isTauri } from "../../../lib/ipc";
  import { errText, settingsStore, toast } from "../../../lib/stores/ui";
  import RunList from "./automation/RunList.svelte";
  import SavedAutomations from "./automation/SavedAutomations.svelte";
  import { RefreshCw, Square } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";

  let { onOpenHotkeys }: { onOpenHotkeys: () => void } = $props();
  let section = $state<"open" | "saved" | "history">("open");
  let snapshot = $state<RunsSnapshot | null>(null);
  let automations = $state<Automation[]>([]);
  let loadError = $state("");
  let now = $state(Date.now());
  let busyId = $state("");
  let disposed = false;
  let version = 0;

  const open = $derived((snapshot?.runs ?? []).filter((r) => r.status === "pending" || r.status === "running")
    .sort((a, b) => a.dueAtMs - b.dueAtMs));
  const history = $derived((snapshot?.runs ?? []).filter((r) => r.status !== "pending" && r.status !== "running"));
  const runningPrograms = $derived(open.filter((r) => r.action.kind === "program").length);
  const stopHotkey = $derived($settingsStore?.automationStopHotkey ?? "");
  const tabs = $derived([
    { id: "open", label: `进行中 ${open.length}` },
    { id: "saved", label: `已保存 ${automations.length}` },
    { id: "history", label: "记录" },
  ] as const);

  async function refresh() {
    const current = ++version;
    try {
      const [runs, saved] = await Promise.all([listRuns(), listAutomations()]);
      if (disposed || current !== version) return;
      snapshot = runs;
      automations = saved;
      loadError = "";
    } catch (error) {
      if (!disposed && current === version) loadError = `读取自动化失败：${errText(error)}`;
    }
  }

  onMount(() => {
    let unlisten: (() => void) | null = null;
    void subscribeAutomations(() => { void refresh(); })
      .then((fn) => { if (disposed) fn(); else unlisten = fn; })
      .catch((error) => { loadError = `自动化状态订阅失败：${errText(error)}`; });
    void refresh();
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => { disposed = true; version++; clearInterval(timer); unlisten?.(); };
  });

  async function stop(run: AutomationRun) {
    busyId = run.id;
    try {
      const count = await stopRuns(run.id);
      if (!count) toast(false, "状态已经变化，请查看最新列表");
      await refresh();
    } catch (error) { toast(false, `操作失败：${errText(error)}`); }
    finally { busyId = ""; }
  }

  async function stopAll() {
    try {
      const count = await stopRuns();
      toast(true, count ? `已停止 ${count} 个自动化` : "没有需要停止的自动化");
      await refresh();
    } catch (error) { toast(false, `停止失败：${errText(error)}`); }
  }
</script>

<SettingsPage
  title="自动化"
  description="琴音替你做的事：提醒、按键、点鼠标、盯屏幕。可以让陪玩或 Agent 生成，也可以自己保存常用的。"
  testid="automation-page"
>
  {#if !isTauri}
    <p class="-mt-2 mb-0 rounded-xl border border-warning/30 bg-warning/8 px-3.5 py-2 text-xs text-warning" role="status">界面预览：以下是示例数据，不会真的运行。</p>
  {/if}
  <nav class="flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1" aria-label="自动化分类">
    {#each tabs as tab}
      {@const active = section === tab.id}
      <button type="button" data-testid={`automation-nav-${tab.id}`} aria-current={active ? "page" : undefined}
        class="h-7 rounded-lg px-3 text-xs font-medium transition-colors
          {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => { section = tab.id; }}>{tab.label}</button>
    {/each}
  </nav>
  {#if loadError}<p role="alert" class="m-0 text-xs text-destructive">{loadError}</p>{/if}
  {#if section === "open"}
    <SettingsSection title={`进行中 · ${open.length}`} id="automation-open-title" flush>
      {#snippet actions()}
        <Button variant="ghost" size="sm" onclick={refresh}><RefreshCw class="size-3.5" />刷新</Button>
        <Button variant="outline" size="sm" disabled={!runningPrograms && !open.some((r) => r.action.kind === "program")}
          onclick={stopAll}><Square class="size-3 fill-current" />停止所有自动化</Button>
      {/snippet}
      <p class="m-0 px-4 py-3 text-xs leading-relaxed text-muted-foreground">
        等待中的提醒和自动化在 Kotone 重启后仍在；退出 Kotone 时运行中的自动化会停下并松开按住的键。
        {#if stopHotkey}在游戏里按 <span class="rounded bg-primary/12 px-1.5 py-0.5 font-mono text-primary">{stopHotkey}</span> 立即停止所有自动化（不影响提醒）。{:else}停止自动化热键未设置。{/if}
        <button type="button" class="text-primary hover:underline" onclick={onOpenHotkeys}>改热键 →</button>
      </p>
      {#if !snapshot}<p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">正在读取…</p>
      {:else if !open.length}<p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">没有进行中的提醒或自动化</p>
      {:else}<RunList runs={open} {now} mode="open" {busyId} onStop={stop} />{/if}
    </SettingsSection>
  {:else if section === "saved"}
    <SavedAutomations {automations} onChanged={refresh} />
  {:else}
    <SettingsSection title="记录" id="automation-history-title" description="最近结束的提醒与自动化（最多 50 条，含失败、取消、错过的原因）。" flush>
      {#if !history.length}<p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">还没有记录</p>
      {:else}<RunList runs={history} {now} mode="history" />{/if}
    </SettingsSection>
  {/if}
</SettingsPage>
