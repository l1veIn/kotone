<script lang="ts">
  // 「自动化」的统一列表：每个自动化一行，像播放器一样 ▶ 运行、■ 停止；带定时的显示“每天 08:00 · 下次 …”；
  // 点开看详情（说明、参数、定时、最近的执行、编辑 / 删除 / 交给琴音）。
  // 不属于任何已保存自动化的执行（琴音设的提醒、她临时写的程序）排在最上面，也能停。
  import { ChevronRight, CircleAlert, Clock, LoaderCircle, MessageSquare, Play, Plus, Square, Zap } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import {
    BUILTIN_REMIND_ID, deleteAutomation, describeSchedule, getAutomation, runAutomation, runTitle,
    type Automation, type AutomationRun,
  } from "../../../../lib/automation";
  import { agentSetAutomationHidden } from "../../../../lib/agent";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import AutomationEditor from "./AutomationEditor.svelte";
  import ArgsForm from "./ArgsForm.svelte";
  import RunList from "./RunList.svelte";

  let { automations, runs, now, busyId, onStop, onChanged }: {
    automations: Automation[];
    runs: AutomationRun[];
    now: number;
    busyId: string;
    onStop: (run: AutomationRun) => void;
    onChanged: () => void;
  } = $props();

  let editing = $state<Automation | null | "new">(null);
  let expanded = $state("");
  let runPanel = $state("");
  let runArgs = $state<Record<string, unknown>>({});
  let switchSeconds = $state(3);
  let starting = $state("");
  let confirmDelete = $state("");
  let offering = $state("");

  const by: Record<Automation["createdBy"], string> = { user: "你", companion: "陪玩", agent: "琴音" };
  const hiddenFromAgent = $derived(new Set($settingsStore?.agent?.hiddenAutomations ?? []));
  const open = (r: AutomationRun) => r.status === "pending" || r.status === "running";
  const automationOf = (r: AutomationRun) => (r.action.kind === "program" ? r.action.automationId ?? null : null);
  /** 不属于已保存自动化的、还没结束的执行。 */
  const loose = $derived(runs.filter((r) => open(r) && r.action.kind !== "chat"
    && !automations.some((a) => a.id === automationOf(r))).sort((a, b) => a.dueAtMs - b.dueAtMs));
  const sorted = $derived([...automations].sort((a, b) => Number(b.id === BUILTIN_REMIND_ID) - Number(a.id === BUILTIN_REMIND_ID)
    || a.name.localeCompare(b.name, "zh-CN")));

  function activeRuns(item: Automation): AutomationRun[] {
    return runs.filter((r) => open(r) && automationOf(r) === item.id);
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
    const hm = d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
    return sameDay ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
  }
  function left(ms: number): string {
    const s = Math.max(0, Math.ceil((ms - now) / 1000));
    return s >= 3600 ? `${Math.floor(s / 3600)} 小时 ${Math.floor((s % 3600) / 60)} 分` : s >= 60 ? `${Math.floor(s / 60)} 分 ${s % 60} 秒` : `${s} 秒`;
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
      toast(true, result.status === "countdown" ? `${result.seconds} 秒后开始，请切到游戏窗口` : "已安排");
      runPanel = "";
      onChanged();
    } catch (error) {
      toast(false, `没能运行：${errText(error)}`);
    } finally {
      starting = "";
    }
  }
  async function edit(item: Automation) {
    try { editing = await getAutomation(item.id); }
    catch (error) { toast(false, `读取失败：${errText(error)}`); }
  }
  async function remove(item: Automation) {
    if (confirmDelete !== item.id) { confirmDelete = item.id; return; }
    confirmDelete = "";
    try { await deleteAutomation(item.id); toast(true, "已删除"); onChanged(); }
    catch (error) { toast(false, `删除失败：${errText(error)}`); }
  }
  async function setOffered(item: Automation, offered: boolean) {
    if (offering) return;
    offering = item.id;
    try {
      const hiddenAutomations = await agentSetAutomationHidden(item.id, !offered);
      settingsStore.update((s) => s && { ...s, agent: { permissions: { mode: "default", rules: {} }, inGame: true, ...s.agent, hiddenAutomations } });
      toast(true, offered ? `「${item.name}」交给琴音使用` : `「${item.name}」不再交给琴音`);
    } catch (error) { toast(false, `保存失败：${errText(error)}`); }
    finally { offering = ""; }
  }
</script>

{#if editing}
  <AutomationEditor automation={editing === "new" ? null : editing}
    onSaved={() => { editing = null; onChanged(); }} onCancel={() => { editing = null; }} />
{:else}
  <section class="min-w-0" data-testid="automation-list">
    <div class="mb-2.5 flex items-end justify-between gap-3 px-1">
      <div>
        <h2 class="m-0 text-sm font-semibold text-foreground">全部自动化 · {automations.length}</h2>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">▶ 运行、■ 停止；带“定时”的到点自己运行。打开“交给琴音”的，她也能帮你运行。</p>
      </div>
      <Button size="sm" onclick={() => { editing = "new"; }} data-testid="automation-new"><Plus class="size-3.5" />新建</Button>
    </div>
    <ul class="m-0 list-none divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline bg-card/70 p-0">
      {#each loose as run (run.id)}
        <li class="flex items-center gap-3 px-4 py-3" data-testid="automation-loose" data-run-id={run.id}>
          <span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
            {#if run.action.kind === "speak"}<MessageSquare class="size-4" />{:else}<Zap class="size-4" />{/if}
          </span>
          <div class="min-w-0 flex-1">
            <p class="m-0 truncate text-[13px] font-medium">{runTitle(run)}</p>
            <p class="m-0 mt-0.5 text-[11px] text-muted-foreground">
              临时{run.action.kind === "speak" ? "提醒" : "程序"} ·
              {#if run.status === "running"}<span class="text-primary">运行中</span>{:else}{clock(run.dueAtMs)}（{left(run.dueAtMs)}后）{/if}
            </p>
          </div>
          <Button size="sm" variant="ghost" aria-label={`停止 ${runTitle(run)}`} disabled={busyId === run.id} onclick={() => onStop(run)} data-testid="automation-stop">
            <Square class="size-3.5 fill-current" />
          </Button>
        </li>
      {/each}
      {#each sorted as item (item.id)}
        {@const active = activeRuns(item)}
        {@const last = lastRun(item)}
        {@const isOpen = expanded === item.id}
        <li data-testid="automation-item" data-automation-id={item.id}>
          <div class="flex items-center gap-3 px-4 py-3">
            {#if active.length}
              <Button size="sm" variant="outline" class="size-8 shrink-0 rounded-full p-0 text-primary" aria-label={`停止 ${item.name}`}
                disabled={busyId === active[0].id} onclick={() => active.forEach(onStop)} data-testid="automation-item-stop">
                <Square class="size-3.5 fill-current" />
              </Button>
            {:else}
              <Button size="sm" variant="outline" class="size-8 shrink-0 rounded-full p-0" aria-label={`运行 ${item.name}`}
                disabled={starting === item.id} onclick={() => play(item)} data-testid="automation-item-play">
                {#if starting === item.id}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Play class="size-3.5 fill-current" />{/if}
              </Button>
            {/if}
            <button type="button" class="min-w-0 flex-1 text-left" aria-expanded={isOpen} onclick={() => { expanded = isOpen ? "" : item.id; }} data-testid="automation-item-toggle">
              <p class="m-0 flex items-center gap-1.5 truncate text-[13px] font-medium text-foreground">
                {item.name}
                {#if item.id === BUILTIN_REMIND_ID}<span class="rounded bg-muted px-1 text-[10px] font-normal text-muted-foreground">内置</span>{/if}
              </p>
              <p class="m-0 mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px] text-muted-foreground">
                <span>{item.action.kind === "speak" ? "播报" : "程序"}</span>
                {#if active.length}<span class="text-primary" data-testid="automation-item-running">运行中</span>{/if}
                {#if item.schedule?.enabled}
                  <span class="flex items-center gap-1 text-foreground/80" data-testid="automation-item-schedule"><Clock class="size-3" />{describeSchedule(item.schedule.cron)}{#if item.nextRunAtMs}{` · 下次 ${clock(item.nextRunAtMs)}`}{/if}</span>
                {/if}
                {#if last && !active.length}
                  <span class:text-destructive={last.status === "failed"}>上次：{last.status === "finished" ? "完成" : last.status === "failed" ? "失败" : last.status === "cancelled" ? "已停止" : "错过"} {clock(last.finishedAtMs ?? last.dueAtMs)}</span>
                {/if}
              </p>
            </button>
            <ChevronRight class="size-4 shrink-0 text-muted-foreground transition-transform {isOpen ? 'rotate-90' : ''}" aria-hidden="true" />
          </div>

          {#if runPanel === item.id}
            <div class="mx-4 mb-3 rounded-xl border border-hairline bg-muted/40 p-3" data-testid="automation-run-panel">
              <ArgsForm params={item.params} values={runArgs} prefix={`${item.name} `} onchange={(values) => { runArgs = values; }} />
              <div class="mt-3 flex flex-wrap items-center justify-end gap-3">
                {#if item.action.kind === "program"}
                  <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
                    点击后
                    <input class="h-7 w-14 rounded-lg border border-input bg-background/60 px-2 text-xs text-foreground outline-none focus:border-ring" type="number" min="1" max="30" bind:value={switchSeconds} aria-label="倒数秒数" />
                    秒在当时的前台窗口（切到游戏）开始
                  </label>
                {/if}
                <Button size="sm" variant="ghost" onclick={() => { runPanel = ""; }}>取消</Button>
                <Button size="sm" disabled={!!starting} onclick={() => void start(item)} data-testid="automation-run-start">{starting ? "启动中…" : "开始"}</Button>
              </div>
            </div>
          {/if}

          {#if isOpen}
            <div class="mx-4 mb-3.5 space-y-3 rounded-xl border border-hairline bg-background/40 p-3.5 text-xs" data-testid="automation-details">
              {#if item.description}<p class="m-0 text-muted-foreground">{item.description}</p>{/if}
              <dl class="m-0 grid grid-cols-[5rem_1fr] gap-x-3 gap-y-1.5">
                <dt class="text-muted-foreground">做什么</dt>
                <dd class="m-0">{item.action.kind === "speak" ? `说“${item.action.text}”（${item.action.delivery.map((d) => (d === "voice" ? "语音" : "弹幕")).join(" + ")}）` : "运行一段程序（按键、点鼠标、看屏幕）"}</dd>
                <dt class="text-muted-foreground">参数</dt>
                <dd class="m-0">{item.params.length ? item.params.map((p) => p.label || p.name).join("、") : "无"}</dd>
                <dt class="text-muted-foreground">定时</dt>
                <dd class="m-0">{#if item.schedule}{item.schedule.enabled ? describeSchedule(item.schedule.cron) : "已暂停"}（{item.schedule.cron}）{:else}不定时，手动或让琴音运行{/if}</dd>
                <dt class="text-muted-foreground">创建</dt>
                <dd class="m-0">{by[item.createdBy]} · {new Date(item.createdAtMs).toLocaleDateString()}</dd>
              </dl>
              <div>
                <p class="m-0 mb-1 text-muted-foreground">最近的执行</p>
                {#if recentRuns(item).length}
                  <div class="overflow-hidden rounded-lg border border-hairline"><RunList runs={recentRuns(item)} {now} mode="history" /></div>
                {:else}<p class="m-0 text-muted-foreground/80">还没有运行过</p>{/if}
              </div>
              <div class="flex flex-wrap items-center gap-2 pt-1">
                <label class="mr-auto flex cursor-pointer items-center gap-1.5 text-muted-foreground">
                  <Switch size="sm" data-testid="saved-automation-agent" checked={!hiddenFromAgent.has(item.id)} disabled={offering === item.id}
                    aria-label={`把「${item.name}」交给琴音使用`} onCheckedChange={(checked) => void setOffered(item, checked)} />
                  交给琴音
                </label>
                <Button variant="outline" size="sm" onclick={() => edit(item)} data-testid="automation-edit">编辑 / 定时</Button>
                <Button variant={confirmDelete === item.id ? "destructive" : "ghost"} size="sm"
                  class={confirmDelete === item.id ? "" : "text-muted-foreground hover:text-destructive"}
                  onclick={() => remove(item)} data-testid="automation-delete">{confirmDelete === item.id ? "确认删除" : "删除"}</Button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
      {#if !automations.length && !loose.length}
        <li class="px-4 py-8 text-center text-[13px] text-muted-foreground"><CircleAlert class="mx-auto mb-1 size-4" />还没有自动化</li>
      {/if}
    </ul>
  </section>
{/if}
