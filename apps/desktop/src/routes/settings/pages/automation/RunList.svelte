<script lang="ts">
  // 执行记录列表：“进行中”（等待中 + 运行中，可取消 / 停止）或“记录”（已结束）。
  import { reminderOf, runTitle, type AutomationRun } from "../../../../lib/automation";
  import { getLocale } from "$lib/paraglide/runtime.js";
  import { m } from "$lib/paraglide/messages.js";

  let { runs, now, mode, busyId = "", onStop }: {
    runs: AutomationRun[]; now: number; mode: "open" | "history"; busyId?: string;
    onStop?: (run: AutomationRun) => void;
  } = $props();

  const statusText: Record<AutomationRun["status"], string> = {
    pending: m.auto_run_status_pending(), running: m.auto_run_status_running(), finished: m.auto_run_status_finished(),
    failed: m.auto_run_status_failed(), cancelled: m.auto_run_status_cancelled(), missed: m.auto_run_status_missed(),
  };
  const outcomeText: Record<string, string> = {
    finished: m.auto_run_outcome_finished(), stopped: m.auto_run_outcome_stopped(), limit: m.auto_run_outcome_limit(), error: m.auto_run_outcome_error(),
  };

  function duration(ms: number): string {
    const seconds = Math.max(0, Math.ceil(ms / 1000));
    const h = Math.floor(seconds / 3600);
    const min = Math.floor((seconds % 3600) / 60);
    return `${h ? `${m.auto_duration_hours({ hours: h })} ` : ""}${min ? `${m.auto_duration_minutes({ minutes: min })} ` : ""}${m.auto_duration_seconds({ seconds: seconds % 60 })}`;
  }

  /** 提醒写回答方式（语音 + 弹幕），绑了窗口的写窗口；动作只有程序一种（ADR 045），不再标种类。 */
  function subtitle(run: AutomationRun): string {
    const reminder = reminderOf(run);
    if (reminder) return reminder.delivery.map((kind) => (kind === "voice" ? m.auto_delivery_voice() : m.auto_delivery_danmaku())).join(" + ");
    return run.action.target?.label ?? "";
  }

  function when(run: AutomationRun): string {
    if (run.status === "pending") {
      const left = run.dueAtMs - now;
      return left <= 0 ? m.auto_run_starting_soon() : m.auto_run_due_in({ duration: duration(left) });
    }
    if (run.status === "running") {
      return reminderOf(run) ? m.auto_run_reminding() : m.auto_run_running_for({ duration: duration(now - (run.startedAtMs ?? now)) });
    }
    return new Date(run.finishedAtMs ?? run.dueAtMs).toLocaleString();
  }
</script>

<ul class="m-0 list-none divide-y divide-hairline p-0">
  {#each runs as run (run.id)}
    {@const stamp = new Date(mode === "history" ? run.finishedAtMs ?? run.dueAtMs : run.startedAtMs ?? run.dueAtMs)}
    <li class="run-row {mode === 'open' && run.status === 'running' ? 'featured' : ''}" data-testid="automation-run" data-status={run.status}>
      <time datetime={stamp.toISOString()} class="text-xs tabular-nums text-muted-foreground" title={stamp.toLocaleString(getLocale())}>
        <span class="block">{stamp.toLocaleTimeString(getLocale(), { hour: "2-digit", minute: "2-digit" })}</span>
        {#if stamp.toDateString() !== new Date(now).toDateString()}<span class="block text-[11px]">{stamp.toLocaleDateString(getLocale(), { month: "short", day: "numeric" })}</span>{/if}
      </time>
      <div class="min-w-0">
        <p class="run-name m-0 text-sm font-medium break-words text-foreground">{runTitle(run)}</p>
        {#if subtitle(run)}<p class="mt-1 mb-0 text-xs text-muted-foreground">{subtitle(run)}</p>{/if}
        {#if mode === "history"}
          {@const args = Object.entries(run.action.args ?? {})}
          {#if run.message}<p class="mt-1 mb-0 text-xs {run.status === 'failed' ? 'text-destructive' : 'text-muted-foreground'}">{run.message}</p>{/if}
          {#if run.report || args.length}
            <details class="mt-1"><summary class="cursor-pointer text-xs text-muted-foreground hover:text-foreground">{m.auto_home_report()}</summary>
              {#if args.length}
                <p class="mt-2 text-xs text-muted-foreground">{m.auto_field_params()}</p>
                <dl class="grid grid-cols-[auto_1fr] gap-x-2 text-xs">
                  {#each args as [key, value]}
                    <dt class="break-all text-muted-foreground">{key}</dt>
                    <dd class="min-w-0 break-words">{typeof value === "string" ? value : JSON.stringify(value)}</dd>
                  {/each}
                </dl>
              {/if}
              {#if run.report}
                <p class="mt-2 text-xs text-muted-foreground">{m.auto_run_report({ outcome: outcomeText[run.report.outcome] ?? run.report.outcome, keys: run.report.keys, duration: duration(run.report.durationMs) })}</p>
                {#if run.report.log.length}
                  <p class="mt-1 text-xs text-muted-foreground">{m.auto_run_log({count:run.report.log.length})}</p>
                  <pre class="mt-1 max-h-40 overflow-auto rounded-md bg-muted/40 p-2 text-xs whitespace-pre-wrap">{run.report.log.join("\n")}</pre>
                {/if}
              {/if}
            </details>
          {/if}
        {/if}
      </div>
      <div class="run-end flex flex-wrap items-center justify-end gap-2">
        <span class="text-xs tabular-nums {run.status === 'failed' ? 'text-destructive' : run.status === 'running' ? 'text-primary' : 'text-muted-foreground'}">{mode === "open" ? when(run) : statusText[run.status]}</span>
        {#if mode === "open" && onStop}
          <button type="button" aria-label={run.status === "running" ? m.auto_list_stop_aria({name:runTitle(run)}) : m.auto_list_cancel_aria({name:runTitle(run)})} class="rounded-md border border-hairline bg-background px-3 py-1 text-xs text-foreground hover:bg-muted disabled:opacity-50" disabled={busyId === run.id} onclick={() => onStop(run)}>
            {run.status === "running" ? m.auto_action_stop() : m.auto_action_cancel()}
          </button>
        {/if}
      </div>
    </li>
  {/each}
</ul>

<style>
  ul { container-type: inline-size; }
  .run-row { display: grid; grid-template-columns: 4.3rem minmax(0, 1fr) auto; align-items: baseline; gap: .4rem .7rem; padding: .85rem .25rem; }
  .featured { padding: .75rem 1rem 1.1rem; }
  .featured .run-name { font-size: 1.05rem; }
  @container (max-width: 420px) {
    .run-row { grid-template-columns: 3.7rem minmax(0, 1fr); }
    .run-end { grid-column: 2; justify-content: start; }
  }
</style>
