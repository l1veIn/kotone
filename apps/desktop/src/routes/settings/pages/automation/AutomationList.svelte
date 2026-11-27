<script lang="ts">
  // 「自动化」的统一列表：每个自动化一行，像播放器一样 ▶ 运行、■ 停止；带定时的显示“每天 08:00 · 下次 …”；
  // 点开看详情（说明、参数、定时、最近的执行、编辑 / 复制 / 删除 / 交给琴音）。
  // 不属于任何已保存自动化的执行（琴音设的提醒、她临时写的程序）排在最上面，也能停。
  // 动作只有程序（ADR 045）：▶ 只对用到游戏窗口的先倒数（让你切到游戏），别的直接运行；等待中的执行（例如内置提醒排上的
  // “打小龙”）挂在它那一项下面，一条一条能取消；■ 只停正在跑的。
  import { companionName } from "$lib/stores/companion";
  import { ChevronRight, CircleAlert, Clock, Gamepad2, LoaderCircle, MessageSquare, Play, Plus, Square, Zap } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import {
    ASKS_PER_MATCH, BUILTIN_LOL_SCOUT_ID, BUILTIN_REMIND_ID, deleteAutomation, describeSchedule, getAutomation, reminderOf, runAutomation, runTitle,
    type Automation, type AutomationRun, type AutomationTemplate,
  } from "../../../../lib/automation";
  import { agentSetAutomationHidden } from "../../../../lib/agent";
  import { describeTrigger, eventCatalog, triggerOff } from "../../../../lib/gameEvents";
  import { pluginsStore } from "../../../../lib/plugins";
  import { listProfiles } from "../../../../lib/ipc";
  import { onMount } from "svelte";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import AutomationEditor from "./AutomationEditor.svelte";
  import ArgsForm from "./ArgsForm.svelte";
  import RunList from "./RunList.svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { listSeparator } from "$lib/locale";
  import { getLocale } from "$lib/paraglide/runtime.js";

  let { automations, templates = [], runs, now, busyId, onStop, onChanged }: {
    automations: Automation[];
    /** 新建时选的模板（随已保存的自动化一起来）。 */
    templates?: AutomationTemplate[];
    runs: AutomationRun[];
    now: number;
    busyId: string;
    onStop: (run: AutomationRun) => void;
    onChanged: () => void;
  } = $props();

  /** 新建（先选模板）、编辑、复制一份（按内置的按键程序改一个自己的）。 */
  let editing = $state<{ mode: "new" } | { mode: "edit" | "copy"; automation: Automation } | null>(null);
  let expanded = $state("");
  let runPanel = $state("");
  let runArgs = $state<Record<string, unknown>>({});
  let switchSeconds = $state(3);
  let starting = $state("");
  let confirmDelete = $state("");
  let offering = $state("");

  // 游戏配置 id → 名字（触发说明里用）
  let profileNames = $state<Record<string, string>>({});
  onMount(async () => {
    try { profileNames = Object.fromEntries((await listProfiles()).map((p) => [p.id, p.displayName])); } catch { profileNames = {}; }
  });
  const gameName = (id: string) => profileNames[id] ?? id;
  // 事件目录（本体的 game.* + 插件登记的）：事件的名字、插件关着时“这个触发不会发生”
  const catalog = $derived(eventCatalog($pluginsStore, $settingsStore));
  /** 详情里“做什么”：用不用游戏窗口（▶ 倒不倒数）、会不会叫她（花 AI 调用）。 */
  function doesText(item: Automation): string {
    return [
      m.auto_list_detail_program(),
      item.usesWindow ? m.auto_list_detail_window() : m.auto_list_detail_no_window(),
      ...(item.asks ? [m.auto_list_detail_asks({ name: $companionName, max: ASKS_PER_MATCH })] : []),
    ].join(" · ");
  }

  const by: Record<Automation["createdBy"], string> = $derived({ user: m.auto_created_by_user(), companion: m.auto_created_by_companion(), agent: $companionName });
  const hiddenFromAgent = $derived(new Set($settingsStore?.agent?.hiddenAutomations ?? []));
  const open = (r: AutomationRun) => r.status === "pending" || r.status === "running";
  const automationOf = (r: AutomationRun) => r.action.automationId ?? null;
  /** 不属于已保存自动化的、还没结束的执行。 */
  const loose = $derived(runs.filter((r) => open(r)
    && !automations.some((a) => a.id === automationOf(r))).sort((a, b) => a.dueAtMs - b.dueAtMs));
  const sorted = $derived([...automations].sort((a, b) => Number(b.id === BUILTIN_REMIND_ID) - Number(a.id === BUILTIN_REMIND_ID)
    || a.name.localeCompare(b.name, getLocale())));

  /** 正在跑的（■ 停的就是这些）。 */
  function activeRuns(item: Automation): AutomationRun[] {
    return runs.filter((r) => r.status === "running" && automationOf(r) === item.id);
  }
  /** 排着、还没到点的（挂在这一项下面，一条一条取消）。 */
  function pendingRuns(item: Automation): AutomationRun[] {
    return runs.filter((r) => r.status === "pending" && automationOf(r) === item.id).sort((a, b) => a.dueAtMs - b.dueAtMs);
  }
  function recentRuns(item: Automation): AutomationRun[] {
    return runs.filter((r) => !open(r) && automationOf(r) === item.id).slice(0, 5);
  }
  function lastRun(item: Automation): AutomationRun | undefined {
    return runs.find((r) => !open(r) && automationOf(r) === item.id);
  }
  function clock(ms: number): string {
    const d = new Date(ms);
    const sameDay = d.toDateString() === new Date(now).toDateString();
    const hm = d.toLocaleTimeString(getLocale(), { hour: "2-digit", minute: "2-digit" });
    return sameDay ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
  }
  function left(ms: number): string {
    const s = Math.max(0, Math.ceil((ms - now) / 1000));
    return s >= 3600 ? `${m.auto_duration_hours({ hours: Math.floor(s / 3600) })} ${m.auto_duration_minutes({ minutes: Math.floor((s % 3600) / 60) })}`
      : s >= 60 ? `${m.auto_duration_minutes({ minutes: Math.floor(s / 60) })} ${m.auto_duration_seconds({ seconds: s % 60 })}`
      : m.auto_duration_seconds({ seconds: s });
  }
  function lastStatusText(run: AutomationRun): string {
    return run.status === "finished" ? m.auto_run_status_finished() : run.status === "failed" ? m.auto_run_status_failed()
      : run.status === "cancelled" ? m.auto_list_last_stopped() : m.auto_list_last_missed();
  }

  function play(item: Automation) {
    // 有参数的先填参数（内置提醒一定要填提醒什么）
    if (item.params.length) {
      runPanel = runPanel === item.id ? "" : item.id;
      runArgs = {};
      return;
    }
    void start(item);
  }
  async function start(item: Automation) {
    if (starting) return;
    starting = item.id;
    try {
      const result = await runAutomation(item.id, runArgs, switchSeconds);
      toast(true, result.status === "countdown" ? m.auto_list_run_countdown({ seconds: result.seconds })
        : result.status === "scheduled" ? m.auto_list_scheduled() : m.auto_list_started());
      runPanel = "";
      onChanged();
    } catch (error) {
      toast(false, m.auto_list_run_failed({ error: errText(error) }));
    } finally {
      starting = "";
    }
  }
  async function edit(item: Automation, mode: "edit" | "copy") {
    try { editing = { mode, automation: await getAutomation(item.id) }; }
    catch (error) { toast(false, m.auto_list_read_failed({ error: errText(error) })); }
  }
  async function remove(item: Automation) {
    if (confirmDelete !== item.id) { confirmDelete = item.id; return; }
    confirmDelete = "";
    try { await deleteAutomation(item.id); toast(true, m.auto_list_deleted()); onChanged(); }
    catch (error) { toast(false, m.auto_list_delete_failed({ error: errText(error) })); }
  }
  async function setOffered(item: Automation, offered: boolean) {
    if (offering) return;
    offering = item.id;
    try {
      const hiddenAutomations = await agentSetAutomationHidden(item.id, !offered);
      settingsStore.update((s) => s && { ...s, agent: { permissions: { mode: "default", rules: {} }, inGame: true, ...s.agent, hiddenAutomations } });
      toast(true, offered ? m.auto_list_offered({ name: item.name, companion: $companionName }) : m.auto_list_not_offered({ name: item.name, companion: $companionName }));
    } catch (error) { toast(false, m.auto_list_save_failed({ error: errText(error) })); }
    finally { offering = ""; }
  }
</script>

{#if editing}
  <AutomationEditor automation={editing.mode === "edit" ? editing.automation : null} copyOf={editing.mode === "copy" ? editing.automation : null}
    {templates} onSaved={() => { editing = null; onChanged(); }} onCancel={() => { editing = null; }} />
{:else}
  <section class="min-w-0" data-testid="automation-list">
    <div class="mb-2.5 flex items-end justify-between gap-3 px-1">
      <div>
        <h2 class="m-0 text-sm font-semibold text-foreground">{m.auto_list_title({ count: automations.length })}</h2>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.auto_list_hint({ companion: $companionName })}</p>
      </div>
      <Button size="sm" onclick={() => { editing = { mode: "new" }; }} data-testid="automation-new"><Plus class="size-3.5" />{m.auto_list_new()}</Button>
    </div>
    <ul class="m-0 list-none divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline bg-card/70 p-0">
      {#each loose as run (run.id)}
        <li class="flex items-center gap-3 px-4 py-3" data-testid="automation-loose" data-run-id={run.id}>
          <span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
            {#if reminderOf(run)}<MessageSquare class="size-4" />{:else}<Zap class="size-4" />{/if}
          </span>
          <div class="min-w-0 flex-1">
            <p class="m-0 truncate text-[13px] font-medium">{runTitle(run)}</p>
            <p class="m-0 mt-0.5 text-[11px] text-muted-foreground">
              {reminderOf(run) ? m.auto_list_loose_reminder() : m.auto_list_loose_program()} ·
              {#if run.status === "running"}<span class="text-primary">{m.auto_run_status_running()}</span>{:else}{m.auto_list_loose_due({ time: clock(run.dueAtMs), left: left(run.dueAtMs) })}{/if}
            </p>
          </div>
          <Button size="sm" variant="ghost" aria-label={m.auto_list_stop_aria({ name: runTitle(run) })} disabled={busyId === run.id} onclick={() => onStop(run)} data-testid="automation-stop">
            <Square class="size-3.5 fill-current" />
          </Button>
        </li>
      {/each}
      {#each sorted as item (item.id)}
        {@const active = activeRuns(item)}
        {@const pending = pendingRuns(item)}
        {@const last = lastRun(item)}
        {@const isOpen = expanded === item.id}
        <li data-testid="automation-item" data-automation-id={item.id}>
          <div class="flex items-center gap-3 px-4 py-3">
            {#if active.length}
              <Button size="sm" variant="outline" class="size-8 shrink-0 rounded-full p-0 text-primary" aria-label={m.auto_list_stop_aria({ name: item.name })}
                disabled={busyId === active[0].id} onclick={() => active.forEach(onStop)} data-testid="automation-item-stop">
                <Square class="size-3.5 fill-current" />
              </Button>
            {:else}
              <Button size="sm" variant="outline" class="size-8 shrink-0 rounded-full p-0" aria-label={m.auto_list_run_aria({ name: item.name })}
                disabled={starting === item.id} onclick={() => play(item)} data-testid="automation-item-play">
                {#if starting === item.id}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Play class="size-3.5 fill-current" />{/if}
              </Button>
            {/if}
            <button type="button" class="min-w-0 flex-1 text-left" aria-expanded={isOpen} onclick={() => { expanded = isOpen ? "" : item.id; }} data-testid="automation-item-toggle">
              <p class="m-0 flex items-center gap-1.5 truncate text-[13px] font-medium text-foreground">
                {item.name}
                {#if item.id === BUILTIN_REMIND_ID || item.id === BUILTIN_LOL_SCOUT_ID}<span class="rounded bg-muted px-1 text-[10px] font-normal text-muted-foreground">{m.auto_list_builtin()}</span>{/if}
                {#if item.asks}<span class="rounded bg-brand-pink/12 px-1 text-[10px] font-normal text-brand-pink" title={m.auto_list_asks_cost()}
                  data-testid="automation-item-asks">{m.auto_list_asks_badge({ name: $companionName })}</span>{/if}
              </p>
              <p class="m-0 mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px] text-muted-foreground">
                {#if item.usesWindow}<span data-testid="automation-item-window">{m.auto_list_uses_window()}</span>{/if}
                {#if active.length}<span class="text-primary" data-testid="automation-item-running">{m.auto_run_status_running()}</span>{/if}
                {#if item.on?.enabled}
                  {@const off = triggerOff(item.on, catalog)}
                  <span class="flex items-center gap-1 text-foreground/80" data-testid="automation-item-trigger"><Gamepad2 class="size-3" />{describeTrigger(item.on, catalog, gameName)}</span>
                  {#if off}<span class="text-warning" data-testid="automation-item-trigger-off">{m.auto_editor_on_event_off({ reason: off })}</span>{/if}
                {/if}
                {#if item.schedule?.enabled}
                  <span class="flex items-center gap-1 text-foreground/80" data-testid="automation-item-schedule"><Clock class="size-3" />{describeSchedule(item.schedule.cron)}{#if item.nextRunAtMs}{` · ${m.auto_list_next_run({ time: clock(item.nextRunAtMs) })}`}{/if}</span>
                {/if}
                {#if last && !active.length && !pending.length}
                  <span class:text-destructive={last.status === "failed"}>{m.auto_list_last_run({ status: lastStatusText(last), time: clock(last.finishedAtMs ?? last.dueAtMs) })}</span>
                {/if}
              </p>
            </button>
            <ChevronRight class="size-4 shrink-0 text-muted-foreground transition-transform {isOpen ? 'rotate-90' : ''}" aria-hidden="true" />
          </div>

          {#each pending as run (run.id)}
            {@const reminder = reminderOf(run)}
            <div class="-mt-1.5 flex items-center gap-2 py-1.5 pr-4 pl-[3.75rem] text-[11px] text-muted-foreground" data-testid="automation-item-pending" data-run-id={run.id}>
              <Clock class="size-3 shrink-0" />
              <span class="min-w-0 truncate text-foreground/85">{reminder ? reminder.text : m.auto_run_status_pending()}</span>
              <span class="shrink-0">· {m.auto_list_loose_due({ time: clock(run.dueAtMs), left: left(run.dueAtMs) })}</span>
              <Button size="sm" variant="ghost" class="ml-auto h-6 px-2 text-[11px]" aria-label={m.auto_list_cancel_aria({ name: runTitle(run) })}
                disabled={busyId === run.id} onclick={() => onStop(run)} data-testid="automation-item-pending-cancel">{m.auto_action_cancel()}</Button>
            </div>
          {/each}

          {#if runPanel === item.id}
            <div class="mx-4 mb-3 rounded-xl border border-hairline bg-muted/40 p-3" data-testid="automation-run-panel">
              <ArgsForm params={item.params} values={runArgs} prefix={`${item.name} `} onchange={(values) => { runArgs = values; }} />
              <div class="mt-3 flex flex-wrap items-center justify-end gap-3">
                {#if item.usesWindow}
                  <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
                    {m.auto_list_countdown_before()}
                    <input class="h-7 w-14 rounded-lg border border-input bg-background/60 px-2 text-xs text-foreground outline-none focus:border-ring" type="number" min="1" max="30" bind:value={switchSeconds} aria-label={m.auto_list_countdown_aria()} />
                    {m.auto_list_countdown_after()}
                  </label>
                {/if}
                <Button size="sm" variant="ghost" onclick={() => { runPanel = ""; }}>{m.auto_action_cancel()}</Button>
                <Button size="sm" disabled={!!starting} onclick={() => void start(item)} data-testid="automation-run-start">{starting ? m.auto_list_starting() : m.auto_list_start()}</Button>
              </div>
            </div>
          {/if}

          {#if isOpen}
            <div class="mx-4 mb-3.5 space-y-3 rounded-xl border border-hairline bg-background/40 p-3.5 text-xs" data-testid="automation-details">
              {#if item.description}<p class="m-0 text-muted-foreground">{item.description}</p>{/if}
              <dl class="m-0 grid grid-cols-[5rem_1fr] gap-x-3 gap-y-1.5">
                <dt class="text-muted-foreground">{m.auto_field_action()}</dt>
                <dd class="m-0" data-testid="automation-detail-does">{doesText(item)}</dd>
                <dt class="text-muted-foreground">{m.auto_field_params()}</dt>
                <dd class="m-0">{item.params.length ? item.params.map((p) => p.label || p.name).join(listSeparator()) : m.auto_list_params_none()}</dd>
                <dt class="text-muted-foreground">{m.auto_field_schedule()}</dt>
                <dd class="m-0">{#if item.schedule}{m.auto_list_schedule_detail({ schedule: item.schedule.enabled ? describeSchedule(item.schedule.cron) : m.auto_list_schedule_paused(), cron: item.schedule.cron })}{:else}{m.auto_list_unscheduled({ companion: $companionName })}{/if}</dd>
                <dt class="text-muted-foreground">{m.auto_field_trigger()}</dt>
                <dd class="m-0" data-testid="automation-detail-trigger">{#if item.on}{item.on.enabled ? describeTrigger(item.on, catalog, gameName) : m.auto_list_trigger_paused({ trigger: describeTrigger(item.on, catalog, gameName) })}{:else}{m.auto_list_no_trigger()}{/if}</dd>
                <dt class="text-muted-foreground">{m.auto_list_created()}</dt>
                <dd class="m-0">{by[item.createdBy]} · {new Date(item.createdAtMs).toLocaleDateString()}</dd>
              </dl>
              <div>
                <p class="m-0 mb-1 text-muted-foreground">{m.auto_list_recent_runs()}</p>
                {#if recentRuns(item).length}
                  <div class="overflow-hidden rounded-lg border border-hairline"><RunList runs={recentRuns(item)} {now} mode="history" /></div>
                {:else}<p class="m-0 text-muted-foreground/80">{m.auto_list_no_runs()}</p>{/if}
              </div>
              <div class="flex flex-wrap items-center gap-2 pt-1">
                <!-- 会叫她的自动化不当工具交给她（ADR 045：免得她调用它再叫她自己） -->
                <label class="mr-auto flex cursor-pointer items-center gap-1.5 text-muted-foreground" class:invisible={item.asks}>
                  <Switch size="sm" data-testid="saved-automation-agent" checked={!hiddenFromAgent.has(item.id)} disabled={offering === item.id}
                    aria-label={m.auto_list_offer_aria({ name: item.name, companion: $companionName })} onCheckedChange={(checked) => void setOffered(item, checked)} />
                  {m.auto_list_offer({ companion: $companionName })}
                </label>
                <Button variant="outline" size="sm" onclick={() => edit(item, "edit")} data-testid="automation-edit">{m.auto_list_edit()}</Button>
                <Button variant="ghost" size="sm" onclick={() => edit(item, "copy")} data-testid="automation-copy">{m.auto_list_copy()}</Button>
                <Button variant={confirmDelete === item.id ? "destructive" : "ghost"} size="sm"
                  class={confirmDelete === item.id ? "" : "text-muted-foreground hover:text-destructive"}
                  onclick={() => remove(item)} data-testid="automation-delete">{confirmDelete === item.id ? m.auto_list_confirm_delete() : m.auto_action_delete()}</Button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
      {#if !automations.length && !loose.length}
        <li class="px-4 py-8 text-center text-[13px] text-muted-foreground"><CircleAlert class="mx-auto mb-1 size-4" />{m.auto_list_empty()}</li>
      {/if}
    </ul>
  </section>
{/if}
