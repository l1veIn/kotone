<!-- 右侧栏里的一件后台的事（ADR 029 / 031）：紧凑版的 BackgroundTaskCard——状态、停止 / 继续、等确认时的允许 / 拒绝。 -->
<!-- 点名字滚到聊天里那张完整的卡片（过程、总结都在那里）。 -->
<script lang="ts">
  import { Check, CircleAlert, CirclePause, LoaderCircle, ShieldAlert } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { agentSubagentAnswer, agentSubagentContinue, agentSubagentStop, type SubagentAnswer, type SubagentInfo } from "../agent";

  let { info, conversationId }: { info: SubagentInfo; conversationId: string } = $props();

  const statusText = $derived.by(() => {
    switch (info.status) {
      case "running": return info.steps > 0 ? `进行中 · 第 ${info.steps} 步` : "进行中";
      case "waiting": return "等你确认";
      case "done": return "做完了";
      case "failed": return "出错了";
      case "stopped": return "停了";
      case "interrupted": return "中断了";
    }
  });
  const tone = $derived(info.status === "done" ? "text-success" : info.status === "failed" ? "text-destructive"
    : info.status === "waiting" ? "text-warning" : info.status === "running" ? "text-primary" : "text-muted-foreground");

  let busy = $state(false);
  let error = $state("");
  async function act(kind: "stop" | "continue" | SubagentAnswer) {
    if (busy || !conversationId) return;
    busy = true;
    error = "";
    try {
      if (kind === "stop") await agentSubagentStop(conversationId, info.name);
      else if (kind === "continue") await agentSubagentContinue(conversationId, info.name);
      else await agentSubagentAnswer(conversationId, info.name, kind);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  /** 滚到聊天里这件事最后那张卡片。 */
  function reveal() {
    const cards = document.querySelectorAll<HTMLElement>(`[data-testid="task-card"][data-task-name="${CSS.escape(info.name)}"]`);
    cards[cards.length - 1]?.scrollIntoView({ block: "center", behavior: "smooth" });
  }
</script>

<li class="min-w-0 rounded-lg border border-hairline bg-card/60 px-2.5 py-2" data-testid="sidebar-task" data-task-name={info.name} data-task-status={info.status}>
  <div class="flex min-w-0 items-center gap-2">
    {#if info.status === "running"}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-primary" />
    {:else if info.status === "waiting"}<ShieldAlert class="size-3.5 shrink-0 text-warning" />
    {:else if info.status === "done"}<Check class="size-3.5 shrink-0 text-success" />
    {:else if info.status === "failed"}<CircleAlert class="size-3.5 shrink-0 text-destructive" />
    {:else}<CirclePause class="size-3.5 shrink-0 text-muted-foreground" />{/if}
    <button type="button" class="min-w-0 flex-1 truncate text-left text-[13px] font-medium text-foreground/90 hover:text-primary"
      title="在聊天里看" data-testid="sidebar-task-label" onclick={reveal}>{info.label || "后台的事"}</button>
    {#if info.status === "running" || info.status === "waiting"}
      <Button size="sm" variant="ghost" class="h-6 shrink-0 px-1.5 text-[11px] text-muted-foreground hover:text-destructive"
        aria-label={`停止「${info.label}」`} data-testid="sidebar-task-stop" disabled={busy} onclick={() => void act("stop")}>停止</Button>
    {:else if info.status === "interrupted"}
      <Button size="sm" variant="ghost" class="h-6 shrink-0 px-1.5 text-[11px] text-primary"
        aria-label={`继续「${info.label}」`} data-testid="sidebar-task-continue" disabled={busy} onclick={() => void act("continue")}>继续</Button>
    {/if}
  </div>
  <p class="m-0 mt-0.5 pl-5.5 text-[11px] {tone}" data-testid="sidebar-task-status">{statusText}</p>
  {#if info.status === "waiting"}
    <div class="mt-1.5 pl-5.5" data-testid="sidebar-task-ask">
      <p class="m-0 text-xs leading-5 text-foreground">{info.question || info.label}</p>
      <div class="mt-1.5 flex gap-1.5">
        <Button size="sm" class="h-6 px-2 text-[11px]" aria-label={`允许「${info.label}」`} data-testid="sidebar-task-allow" disabled={busy} onclick={() => void act("allow-once")}>允许</Button>
        <Button size="sm" variant="ghost" class="h-6 px-2 text-[11px] text-destructive hover:bg-destructive/10 hover:text-destructive"
          aria-label={`拒绝「${info.label}」`} data-testid="sidebar-task-deny" disabled={busy} onclick={() => void act("deny")}>拒绝</Button>
      </div>
    </div>
  {:else if (info.status === "done" || info.status === "failed") && info.summary}
    <p class="m-0 mt-0.5 line-clamp-2 pl-5.5 text-xs leading-5 text-muted-foreground" data-testid="sidebar-task-summary">{info.summary}</p>
  {/if}
  {#if error}<p role="alert" class="m-0 mt-1 pl-5.5 text-[11px] text-destructive">{error}</p>{/if}
</li>
