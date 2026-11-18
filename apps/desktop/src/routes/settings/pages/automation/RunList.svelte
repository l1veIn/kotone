<script lang="ts">
  // 执行记录列表：“进行中”（等待中 + 运行中，可取消 / 停止）或“记录”（已结束）。
  import { runTitle, type AutomationRun } from "../../../../lib/automation";

  let { runs, now, mode, busyId = "", onStop }: {
    runs: AutomationRun[]; now: number; mode: "open" | "history"; busyId?: string;
    onStop?: (run: AutomationRun) => void;
  } = $props();

  const statusText: Record<AutomationRun["status"], string> = {
    pending: "等待中", running: "运行中", finished: "完成", failed: "失败", cancelled: "已取消", missed: "已错过",
  };
  const outcomeText: Record<string, string> = { finished: "跑完", stopped: "被停止", limit: "超出上限", error: "出错" };

  function duration(ms: number): string {
    const seconds = Math.max(0, Math.ceil(ms / 1000));
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return `${h ? `${h} 小时 ` : ""}${m ? `${m} 分 ` : ""}${seconds % 60} 秒`;
  }

  function subtitle(run: AutomationRun): string {
    if (run.action.kind === "speak") {
      return run.action.delivery.map((kind) => (kind === "voice" ? "语音" : "弹幕")).join(" + ");
    }
    if (run.action.kind === "chat") return run.action.sent ? "替你发到了游戏聊天" : "打进了聊天框（未发送）";
    return run.action.target?.label ?? "";
  }

  const badgeText = { speak: "提醒", program: "自动化", chat: "打字" } as const;

  function when(run: AutomationRun): string {
    if (run.status === "pending") {
      const left = run.dueAtMs - now;
      return left <= 0 ? "即将开始" : `${duration(left)}后`;
    }
    if (run.status === "running") {
      return run.action.kind === "speak" ? "正在提醒" : `已运行 ${duration(now - (run.startedAtMs ?? now))}`;
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
            <p class="mt-1 mb-0 text-xs text-muted-foreground">{outcomeText[run.report.outcome] ?? run.report.outcome} · 输入 {run.report.keys} 次 · 用时 {duration(run.report.durationMs)}</p>
            {#if run.report.log.length}
              <details class="mt-1"><summary class="cursor-pointer text-[11px] text-muted-foreground hover:text-foreground">日志（{run.report.log.length}）</summary>
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
            {run.status === "running" ? "停止" : "取消"}
          </button>
        {/if}
      </div>
    </li>
  {/each}
</ul>
