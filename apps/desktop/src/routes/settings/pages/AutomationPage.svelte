<script lang="ts">
  // 「自动化」页（ADR 013 §8）：一个列表——每个自动化 ▶ 运行 / ■ 停止、定时、详情；临时的提醒与程序排在最上面；
  // 下面可以展开全部记录。
  import { companionName } from "$lib/stores/companion";
  import { onMount } from "svelte";
  import {
    listAutomations, listRuns, stopRuns, subscribeAutomations,
    type Automation, type AutomationRun, type RunsSnapshot,
  } from "../../../lib/automation";
  import { isTauri } from "../../../lib/ipc";
  import { errText, settingsStore, toast } from "../../../lib/stores/ui";
  import RunList from "./automation/RunList.svelte";
  import AutomationList from "./automation/AutomationList.svelte";
  import { RefreshCw, Square } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenHotkeys }: { onOpenHotkeys: () => void } = $props();
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

  async function refresh() {
    const current = ++version;
    try {
      const [runs, saved] = await Promise.all([listRuns(), listAutomations()]);
      if (disposed || current !== version) return;
      snapshot = runs;
      automations = saved;
      loadError = "";
    } catch (error) {
      if (!disposed && current === version) loadError = m.auto_page_load_failed({ error: errText(error) });
    }
  }

  onMount(() => {
    let unlisten: (() => void) | null = null;
    void subscribeAutomations(() => { void refresh(); })
      .then((fn) => { if (disposed) fn(); else unlisten = fn; })
      .catch((error) => { loadError = m.auto_page_subscribe_failed({ error: errText(error) }); });
    void refresh();
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => { disposed = true; version++; clearInterval(timer); unlisten?.(); };
  });

  async function stop(run: AutomationRun) {
    busyId = run.id;
    try {
      const count = await stopRuns(run.id);
      if (!count) toast(false, m.auto_page_state_changed());
      await refresh();
    } catch (error) { toast(false, m.auto_page_action_failed({ error: errText(error) })); }
    finally { busyId = ""; }
  }

  async function stopAll() {
    try {
      const count = await stopRuns();
      toast(true, count ? m.auto_page_stopped_count({ count }) : m.auto_page_nothing_to_stop());
      await refresh();
    } catch (error) { toast(false, m.auto_page_stop_failed({ error: errText(error) })); }
  }
</script>

<SettingsPage
  title={m.auto_page_title()}
  description={m.auto_page_description({ companion: $companionName })}
  testid="automation-page"
>
  {#if !isTauri}
    <p class="-mt-2 mb-0 rounded-xl border border-warning/30 bg-warning/8 px-3.5 py-2 text-xs text-warning" role="status">{m.auto_page_preview_notice()}</p>
  {/if}
  {#if loadError}<p role="alert" class="m-0 text-xs text-destructive">{loadError}</p>{/if}
  <div class="flex flex-wrap items-center gap-2 rounded-xl border border-hairline bg-card/50 px-3.5 py-2.5 text-xs text-muted-foreground" data-testid="automation-status">
    <span class="mr-auto">
      {#if open.length}<span class="text-foreground">{m.auto_page_open_count({ count: open.length })}</span> · {/if}
      {#if stopHotkey}{m.auto_page_stop_hotkey_before()} <span class="rounded bg-primary/12 px-1.5 py-0.5 font-mono text-primary">{stopHotkey}</span> {m.auto_page_stop_hotkey_after()}{:else}{m.auto_page_stop_hotkey_unset()}{/if}
      <button type="button" class="ml-1 text-primary hover:underline" onclick={onOpenHotkeys}>{m.auto_page_change_hotkey()}</button>
    </span>
    <Button variant="ghost" size="sm" onclick={refresh}><RefreshCw class="size-3.5" />{m.auto_page_refresh()}</Button>
    <Button variant="outline" size="sm" disabled={!runningPrograms && !open.some((r) => r.action.kind === "program")}
      onclick={stopAll} data-testid="automation-stop-all"><Square class="size-3 fill-current" />{m.auto_page_stop_all()}</Button>
  </div>
  {#if !snapshot}<p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">{m.auto_page_loading()}</p>
  {:else}
    <AutomationList {automations} runs={snapshot.runs} {now} {busyId} onStop={stop} onChanged={refresh} />
    <details class="group" data-testid="automation-history">
      <summary class="cursor-pointer list-none px-1 text-sm font-semibold text-foreground">
        {m.auto_page_history_title({ count: history.length })} <span class="text-xs font-normal text-muted-foreground">{m.auto_page_history_hint()}</span>
      </summary>
      <div class="mt-2.5 overflow-hidden rounded-2xl border border-hairline bg-card/70">
        {#if !history.length}<p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">{m.auto_page_history_empty()}</p>
        {:else}<RunList runs={history} {now} mode="history" />{/if}
      </div>
    </details>
  {/if}
</SettingsPage>
