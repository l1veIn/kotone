<!-- 右侧栏里的一件后台的事（ADR 029 / 031）：紧凑版的 BackgroundTaskCard——状态、停止 / 继续、等确认时的允许 / 拒绝。 -->
<!-- 点开就在侧栏里展开看结果和一步步的过程（和聊天里的卡片共用 TaskProcess）；小按钮滚到聊天里那张卡片。 -->
<script lang="ts">
  import { Check, ChevronRight, CircleAlert, CirclePause, LoaderCircle, MessageSquareText, ShieldAlert } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import { agentSubagentAnswer, agentSubagentContinue, agentSubagentStop, type SubagentAnswer, type SubagentInfo } from "../agent";
  import TaskProcess from "./TaskProcess.svelte";

  let { info, conversationId, open = $bindable() }: {
    info: SubagentInfo;
    conversationId: string;
    /** 展开看过程（右侧栏据此给任务区多留地方；没展开过时是 undefined）。 */
    open?: boolean;
  } = $props();

  const statusText = $derived.by(() => {
    switch (info.status) {
      case "running": return info.steps > 0 ? m.chat_task_status_running_steps({ steps: info.steps }) : m.chat_task_status_running();
      case "waiting": return m.chat_task_status_waiting();
      case "done": return m.chat_task_status_done();
      case "failed": return m.chat_task_status_failed();
      case "stopped": return m.chat_task_status_stopped();
      case "interrupted": return m.chat_task_status_interrupted();
    }
  });
  const tone = $derived(info.status === "done" ? "text-success" : info.status === "failed" ? "text-destructive"
    : info.status === "waiting" ? "text-warning" : info.status === "running" ? "text-primary" : "text-muted-foreground");
  const finished = $derived(info.status === "done" || info.status === "failed");

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

  /** 聊天里有这件事的卡片（旧对话、别的入口派的可能没有）。 */
  const inChat = () => document.querySelectorAll(`[data-testid="task-card"][data-task-name="${CSS.escape(info.name)}"]`).length > 0;
  /** 滚到聊天里这件事最后那张卡片。 */
  function reveal() {
    const cards = document.querySelectorAll<HTMLElement>(`[data-testid="task-card"][data-task-name="${CSS.escape(info.name)}"]`);
    cards[cards.length - 1]?.scrollIntoView({ block: "center", behavior: "smooth" });
  }
</script>

<!-- 右侧栏“卡片”按列排：点开看过程时占满一行 -->
<li class="min-w-0 self-start overflow-hidden rounded-xl border border-hairline bg-card/60 {open ? 'col-span-full' : ''}"
  data-testid="sidebar-task" data-task-name={info.name} data-task-status={info.status} data-open={open ? "true" : "false"}>
  <div class="flex min-w-0 items-center gap-1 py-1.5 pr-1 pl-2.5">
    <button type="button" class="flex min-w-0 flex-1 items-center gap-2 rounded-md text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      aria-expanded={!!open} title={open ? m.chat_sidebar_task_collapse() : m.chat_sidebar_task_expand()} data-testid="sidebar-task-toggle" onclick={() => { open = !open; }}>
      {#if info.status === "running"}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-primary" />
      {:else if info.status === "waiting"}<ShieldAlert class="size-3.5 shrink-0 text-warning" />
      {:else if info.status === "done"}<Check class="size-3.5 shrink-0 text-success" />
      {:else if info.status === "failed"}<CircleAlert class="size-3.5 shrink-0 text-destructive" />
      {:else}<CirclePause class="size-3.5 shrink-0 text-muted-foreground" />{/if}
      <span class="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground/90" data-testid="sidebar-task-label">{info.label || m.chat_sidebar_task_default_label()}</span>
      <ChevronRight aria-hidden="true" class="size-3.5 shrink-0 text-muted-foreground transition-transform {open ? 'rotate-90' : ''}" />
    </button>
    <Button size="icon" variant="ghost" class="size-6 shrink-0 text-muted-foreground" aria-label={m.chat_sidebar_task_reveal_label({ label: info.label })} title={m.chat_sidebar_task_reveal()}
      data-testid="sidebar-task-reveal" onclick={() => { if (inChat()) reveal(); else open = true; }}><MessageSquareText class="size-3.5" /></Button>
    {#if info.status === "running" || info.status === "waiting"}
      <Button size="sm" variant="ghost" class="h-6 shrink-0 px-1.5 text-[11px] text-muted-foreground hover:text-destructive"
        aria-label={m.chat_sidebar_task_stop_label({ label: info.label })} data-testid="sidebar-task-stop" disabled={busy} onclick={() => void act("stop")}>{m.chat_task_stop()}</Button>
    {:else if info.status === "interrupted"}
      <Button size="sm" variant="ghost" class="h-6 shrink-0 px-1.5 text-[11px] text-primary"
        aria-label={m.chat_sidebar_task_continue_label({ label: info.label })} data-testid="sidebar-task-continue" disabled={busy} onclick={() => void act("continue")}>{m.chat_task_continue()}</Button>
    {/if}
  </div>
  <p class="m-0 -mt-1 pb-1.5 pl-8 text-[11px] {tone}" data-testid="sidebar-task-status">{statusText}</p>
  {#if info.status === "waiting"}
    <div class="pb-2 pl-8 pr-2.5" data-testid="sidebar-task-ask">
      <p class="m-0 text-xs leading-5 text-foreground">{info.question || info.label}</p>
      <div class="mt-1.5 flex gap-1.5">
        <Button size="sm" class="h-6 px-2 text-[11px]" aria-label={m.chat_sidebar_task_allow_label({ label: info.label })} data-testid="sidebar-task-allow" disabled={busy} onclick={() => void act("allow-once")}>{m.chat_ask_allow_once()}</Button>
        <Button size="sm" variant="ghost" class="h-6 px-2 text-[11px] text-destructive hover:bg-destructive/10 hover:text-destructive"
          aria-label={m.chat_sidebar_task_deny_label({ label: info.label })} data-testid="sidebar-task-deny" disabled={busy} onclick={() => void act("deny")}>{m.chat_ask_deny()}</Button>
      </div>
    </div>
  {:else if finished && info.summary && !open}
    <p class="m-0 line-clamp-2 pb-2 pl-8 pr-2.5 text-xs leading-5 text-muted-foreground" data-testid="sidebar-task-summary">{info.summary}</p>
  {/if}
  {#if error}<p role="alert" class="m-0 pb-1.5 pl-8 text-[11px] text-destructive">{error}</p>{/if}
  {#if open}
    <div class="max-h-80 min-w-0 space-y-2 overflow-auto border-t border-hairline bg-muted/30 px-2.5 py-2" data-testid="sidebar-task-body">
      <TaskProcess {conversationId} name={info.name} {info} summary={info.summary ?? ""} showResult={finished}
        running={info.status === "running" || info.status === "waiting"} />
    </div>
  {/if}
</li>
