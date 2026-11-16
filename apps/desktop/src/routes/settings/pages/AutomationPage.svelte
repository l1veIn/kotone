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
  import "./panel.css";

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

<div class="panel-page px-6 py-5" data-testid="automation-page">
  <h1 class="text-lg font-bold">自动化</h1>
  <p class="mt-1 text-xs leading-relaxed text-white/50">
    琴音替你做的事：提醒、按键、点鼠标、盯屏幕。可以让陪玩或 Agent 生成，也可以自己保存常用的。
  </p>
  {#if !isTauri}<p class="notice text-amber-200" role="status">界面预览：以下是示例数据，不会真的运行。</p>{/if}
  <nav class="mt-4 flex flex-wrap gap-1.5" aria-label="自动化分类">
    {#each tabs as tab}
      {@const active = section === tab.id}
      <button type="button" data-testid={`automation-nav-${tab.id}`} aria-current={active ? "page" : undefined}
        class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition
          {active ? 'bg-kotone-cyan/15 text-kotone-cyan ring-kotone-cyan/40' : 'bg-white/5 text-white/55 ring-white/10 hover:bg-white/10 hover:text-white/80'}"
        onclick={() => { section = tab.id; }}>{tab.label}</button>
    {/each}
  </nav>
  <div class="mt-4">
    {#if loadError}<p role="alert" class="notice text-kotone-pink">{loadError}</p>{/if}
    {#if section === "open"}
      <section class="panel" aria-labelledby="automation-open-title">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <h2 id="automation-open-title">进行中 <span class="ml-1 text-kotone-cyan">{open.length}</span></h2>
          <div class="flex gap-2">
            <button type="button" class="secondary" onclick={refresh}>刷新</button>
            <button type="button" class="secondary" disabled={!runningPrograms && !open.some((r) => r.action.kind === "program")}
              onclick={stopAll}>停止所有自动化</button>
          </div>
        </div>
        <p class="mt-2 text-xs leading-relaxed text-white/45">
          等待中的提醒和自动化在 Kotone 重启后仍在；退出 Kotone 时运行中的自动化会停下并松开按住的键。
          {#if stopHotkey}在游戏里按 <span class="text-kotone-cyan">{stopHotkey}</span> 立即停止所有自动化（不影响提醒）。{:else}停止自动化热键未设置。{/if}
          <button type="button" class="text-kotone-cyan hover:underline" onclick={onOpenHotkeys}>改热键 →</button>
        </p>
        {#if !snapshot}<p class="empty">正在读取…</p>
        {:else if !open.length}<p class="empty">没有进行中的提醒或自动化</p>
        {:else}<RunList runs={open} {now} mode="open" {busyId} onStop={stop} />{/if}
      </section>
    {:else if section === "saved"}
      <SavedAutomations {automations} onChanged={refresh} />
    {:else}
      <section class="panel" aria-labelledby="automation-history-title">
        <h2 id="automation-history-title">记录</h2>
        <p class="mt-2 text-xs text-white/45">最近结束的提醒与自动化（最多 50 条，含失败、取消、错过的原因）。</p>
        {#if !history.length}<p class="empty">还没有记录</p>
        {:else}<RunList runs={history} {now} mode="history" />{/if}
      </section>
    {/if}
  </div>
</div>
