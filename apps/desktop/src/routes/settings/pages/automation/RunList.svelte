<script lang="ts">
  // 执行记录列表：“进行中”（等待中 + 运行中，可取消 / 停止）或“记录”（已结束）。
  import { runTitle, type AutomationRun } from "../../../../lib/automation";
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

  function subtitle(run: AutomationRun): string {
    if (run.action.kind === "speak") {
      return run.action.delivery.map((kind) => (kind === "voice" ? m.auto_delivery_voice() : m.auto_delivery_danmaku())).join(" + ");
    }
    return run.action.target?.label ?? "";
  }

  const badgeText = { speak: m.auto_kind_speak(), program: m.auto_kind_program() } as const;

  function when(run: AutomationRun): string {
    if (run.status === "pending") {
      const left = run.dueAtMs - now;
      return left <= 0 ? m.auto_run_starting_soon() : m.auto_run_due_in({ duration: duration(left) });
    }
    if (run.status === "running") {
      return run.action.kind === "speak" ? m.auto_run_reminding() : m.auto_run_running_for({ duration: duration(now - (run.startedAtMs ?? now)) });
    }
    return new Date(run.finishedAtMs ?? run.dueAtMs).toLocaleString();
  }
</script>

<ul class="m-0 list-none divide-y divide-hairline p-0">
  {#each runs as run (run.id)}
    <li class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-3" data-testid="automation-run" data-status={run.status}>
      <div class="min-w-0 flex-1">
        <p class="m-0 text-[13px] break-words {mode === 'open' ? 'text-foreground' : 'text-foreground/80'}">
          <span class="mr-1.5 inline-block rounded-md px-1.5 text-[11px] leading-[18px] align-[1px] {run.action.kind === 'program' ? 'bg-primary/12 text-primary' : 'bg-brand-pink/12 text-brand-pink'}">{badgeText[run.action.kind]}</span>
          {runTitle(run)}
        </p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">{[subtitle(run), mode === "open" ? new Date(run.dueAtMs).toLocaleString() : ""].filter(Boolean).join(" · ")}</p>
        {#if mode === "history"}
          {#if run.message}<p class="mt-1 mb-0 text-xs {run.status === 'failed' ? 'text-destructive' : 'text-muted-foreground'}">{run.message}</p>{/if}
          {#if run.report}
            <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.auto_run_report({ outcome: outcomeText[run.report.outcome] ?? run.report.outcome, keys: run.report.keys, duration: duration(run.report.durationMs) })}</p>
            {#if run.report.log.length}
              <details class="mt-1"><summary class="cursor-pointer text-[11px] text-muted-foreground hover:text-foreground">{m.auto_run_log({ count: run.report.log.length })}</summary>
                <pre class="mt-1 mb-0 max-h-40 overflow-auto rounded-lg border border-hairline bg-muted/40 p-2 font-mono text-[11px] whitespace-pre-wrap text-foreground/80">{run.report.log.join("\n")}</pre>
              </details>
            {/if}
          {/if}
        {/if}
      </div>
      <div class="flex shrink-0 items-center gap-3">
        <span class="text-xs tabular-nums {mode === 'open' ? 'text-primary' : 'text-muted-foreground'}">{mode === "open" ? when(run) : `${statusText[run.status]} · ${when(run)}`}</span>
        {#if mode === "open" && onStop}
          <button type="button" class="h-7 rounded-lg border border-hairline px-2.5 text-xs text-foreground transition-colors hover:bg-muted disabled:opacity-50" disabled={busyId === run.id} onclick={() => onStop(run)}>
            {run.status === "running" ? m.auto_action_stop() : m.auto_action_cancel()}
          </button>
        {/if}
      </div>
    </li>
  {/each}
</ul>
