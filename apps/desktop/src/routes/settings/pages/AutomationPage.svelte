<script lang="ts">
  import { companionName } from "$lib/stores/companion";
  import { onMount } from "svelte";
  import {
    describeSchedule, listRuns, listSaved, stopRuns, subscribeAutomations,
    type Automation, type AutomationRun, type AutomationTemplate, type RunsSnapshot,
  } from "../../../lib/automation";
  import { describeTrigger, eventCatalog, triggerOff } from "$lib/gameEvents";
  import { pluginsStore } from "$lib/plugins";
  import { getLocale } from "$lib/paraglide/runtime.js";
  import { isTauri, listProfiles } from "../../../lib/ipc";
  import { errText, settingsStore, toast } from "../../../lib/stores/ui";
  import RunList from "./automation/RunList.svelte";
  import AutomationList from "./automation/AutomationList.svelte";
  import { Activity, ArrowLeft, ChevronRight, Clock, Library, RefreshCw, Square, Zap } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenHotkeys }: { onOpenHotkeys: () => void } = $props();
  let snapshot = $state<RunsSnapshot | null>(null);
  let automations = $state<Automation[]>([]);
  let templates = $state<AutomationTemplate[]>([]);
  let loadError = $state("");
  let now = $state(Date.now());
  let busyId = $state("");
  let disposed = false;
  let version = 0;

  let historyCount = $state(5);
  let library = $state(false);
  let expanded = $state("");
  let profileNames = $state<Record<string, string>>({});
  const catalog = $derived(eventCatalog($pluginsStore, $settingsStore));
  const rules = $derived(automations.filter(a => a.schedule?.enabled || a.on?.enabled));
  function showRule(id: string) {
    expanded = id;
    library = true;
  }

  const open = $derived((snapshot?.runs ?? []).filter((r) => r.status === "pending" || r.status === "running")
    .sort((a, b) => a.dueAtMs - b.dueAtMs));
  const active = $derived(open.filter(r => r.status === "running"));
  const pending = $derived(open.filter(r => r.status === "pending"));
  const history = $derived((snapshot?.runs ?? []).filter((r) => r.status !== "pending" && r.status !== "running")
    .sort((a,b) => (b.finishedAtMs ?? b.dueAtMs) - (a.finishedAtMs ?? a.dueAtMs)));
  // 停下（ADR 045 D6）停掉运行中的、取消等待中绑了游戏窗口的；只剩等待中的提醒时没什么可停
  const stoppable = $derived(open.some((r) => r.status === "running" || !!r.action.target));
  const stopHotkey = $derived($settingsStore?.automationStopHotkey ?? "");

  async function refresh() {
    const current = ++version;
    try {
      const [runs, saved] = await Promise.all([listRuns(), listSaved()]);
      if (disposed || current !== version) return;
      snapshot = runs;
      automations = saved.automations;
      templates = saved.templates ?? [];
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
    void listProfiles().then(profiles => { if (!disposed) profileNames = Object.fromEntries(profiles.map(p => [p.id, p.displayName])); }).catch(() => {});
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
  title={library ? m.auto_home_library() : m.auto_page_title()}
  description={library ? "" : m.auto_page_description({ companion: $companionName })}
  testid="automation-page"
>
  {#snippet actions()}
    <Button variant="ghost" size="sm" aria-label={m.auto_page_refresh()} title={m.auto_page_refresh()} onclick={refresh}><RefreshCw class="size-3.5" /></Button>
    <Button variant="outline" size="sm" data-testid="automation-library" onclick={() => { expanded = ""; library = !library; }}>
      {#if library}<ArrowLeft class="size-4" />{m.auto_home_activity()}{:else}<Library class="size-4" />{m.auto_home_library()}{/if}
    </Button>
  {/snippet}
  {#if loadError}<p role="alert" class="m-0 text-xs text-destructive">{loadError}</p>{/if}
  {#if !snapshot}<p class="text-sm text-muted-foreground">{m.auto_page_loading()}</p>
  {:else if library}
    <AutomationList bind:expanded {automations} {templates} runs={snapshot.runs} {now} {busyId} onStop={stop} onChanged={refresh} />
  {:else}
    {#if active.length}
      <section class="rounded-xl border border-primary/25 bg-primary/5" data-testid="automation-active">
        <h2 class="flex items-center gap-2 px-4 pt-4 text-xs font-semibold text-primary"><Activity class="size-4" />{m.auto_home_running()}</h2>
        <RunList runs={active} {now} mode="open" {busyId} onStop={stop} />
      </section>
    {/if}
    {#if pending.length}
      <section data-testid="automation-pending">
        <h2 class="home-title"><Clock class="size-4 text-muted-foreground" />{m.auto_home_pending()}</h2>
        <RunList runs={pending} {now} mode="open" {busyId} onStop={stop} />
      </section>
    {/if}
    <section data-testid="automation-history">
      <h2 class="home-title">{m.auto_page_history_title()}</h2>
      {#if history.length}<RunList runs={history.slice(0, historyCount)} {now} mode="history" />
        {#if history.length > historyCount}<Button variant="ghost" size="sm" onclick={() => { historyCount += 10; }}>{m.auto_home_older()}</Button>{/if}
      {:else}<p class="my-6 text-sm text-muted-foreground">{m.auto_page_history_empty()}</p>{/if}
    </section>
    {#if rules.length}
      <section>
        <h2 class="home-title">{m.auto_home_rules()}</h2>
        {#each rules as item (item.id)}
          <button class="flex w-full items-center gap-3 border-b border-hairline py-3 text-left hover:bg-muted/40" onclick={() => showRule(item.id)}>
            <Zap class="size-4 shrink-0 text-muted-foreground" />
            <span class="min-w-0 flex-1"><strong class="block text-[13px] font-medium">{item.name}</strong>
              {#if item.schedule?.enabled}<span class="block text-xs text-muted-foreground">{describeSchedule(item.schedule.cron)}{#if item.nextRunAtMs} · {m.auto_list_next_run({time: new Date(item.nextRunAtMs).toLocaleString(getLocale(), {month:"numeric",day:"numeric",hour:"2-digit",minute:"2-digit"})})}{/if}</span>{/if}
              {#if item.on?.enabled}
                {@const off = triggerOff(item.on, catalog)}
                <span class="block text-xs text-muted-foreground">{describeTrigger(item.on,catalog,id => profileNames[id] ?? id)}</span>
                {#if off}<span class="text-xs text-warning">{m.auto_editor_on_event_off({reason:off})}</span>{/if}
              {/if}
            </span><ChevronRight class="size-4 shrink-0 text-muted-foreground" />
          </button>
        {/each}
      </section>
    {/if}
  {/if}
  <div class="flex flex-wrap items-center gap-2 border-t border-hairline pt-4 text-xs text-muted-foreground" data-testid="automation-status">
    <span class="mr-auto">
      {#if stopHotkey}{m.auto_page_stop_hotkey_before()} <span class="rounded bg-primary/12 px-1.5 py-0.5 font-mono text-primary">{stopHotkey}</span> {m.auto_page_stop_hotkey_after()}{:else}{m.auto_page_stop_hotkey_unset()}{/if}
      <button type="button" class="ml-1 text-primary hover:underline" onclick={onOpenHotkeys}>{m.auto_page_change_hotkey()}</button>
    </span>
    <Button variant="outline" size="sm" disabled={!stoppable}
      onclick={stopAll} data-testid="automation-stop-all"><Square class="size-3 fill-current" />{m.auto_page_stop_all()}</Button>
  </div>
  {#if !isTauri}<p class="text-xs text-muted-foreground" role="status">{m.auto_page_preview_notice()}</p>{/if}
</SettingsPage>

<style>
  .home-title { display: flex; align-items: center; gap: .5rem; margin: 0; padding-bottom: .625rem; border-bottom: 1px solid var(--color-hairline); font-size: .875rem; font-weight: 600; }
</style>
