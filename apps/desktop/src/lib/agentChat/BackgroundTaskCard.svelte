<!-- 交给后台做的一件事（ADR 029 第三节）：主会话里 agent.start / agent.followup 的那一行显示成这张卡片。 -->
<!-- 主会话里超过 3 秒转到后台的工具（ADR 029 §一）也是这张卡片：标题照工具卡片的叫法（“查战绩 · 进行中”→ 完成）。 -->
<!-- 玩家看不到“另一段对话”，只看到“写喝药脚本 · 进行中 [停止]”；展开看它一步步做了什么（只读，复用工具卡片）。 -->
<!-- 等确认时的三个按钮沿用 ToolCallCard 的确认卡片（文案与样式）。Kotone 自己的实现。 -->
<script lang="ts">
  import { Check, ChevronRight, CircleAlert, CirclePause, CircleStop, ClipboardList, LoaderCircle, Play, ShieldAlert } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import {
    agentSubagentAnswer, agentSubagentContinue, agentSubagentStop, movedTaskName, toolLabel,
    type SubagentAnswer, type SubagentInfo, type SubagentStatus,
  } from "../agent";
  import { pluginToolLabel } from "../pluginUi";
  import { tryAgentChat } from "./context";
  import type { ToolView } from "./timeline";
  import TaskProcess from "./TaskProcess.svelte";

  let { tool, runActive = false }: { tool: ToolView; runActive?: boolean } = $props();

  const chat = tryAgentChat();

  function parse(text: string | null | undefined): Record<string, unknown> {
    try { const value = JSON.parse(text ?? ""); return value && typeof value === "object" ? value : {}; } catch { return {}; }
  }
  const args = $derived(parse(tool.call.arguments));
  const result = $derived(parse(tool.content));
  /** 超过 3 秒转到后台的工具：后台那件事的名字在结果里（参数是工具自己的）。 */
  const moved = $derived(movedTaskName(tool.content));
  const name = $derived(moved ?? (typeof args.name === "string" ? args.name : ""));
  const conversationId = $derived(chat?.conversation?.id ?? "");
  const info = $derived<SubagentInfo | undefined>(name ? chat?.subagent(name) : undefined);

  /** 同一件事后来又接着做过（agent.followup）：只有最后那张卡片能操作、显示现在的状态。 */
  const latest = $derived.by(() => {
    const messages = chat?.conversation?.messages ?? [];
    let last: string | null = null;
    for (const m of messages) {
      if (m.role !== "assistant") continue;
      for (const call of m.toolCalls ?? []) {
        if ((call.name === "agent.start" || call.name === "agent.followup") && parse(call.arguments).name === name) last = call.id;
      }
    }
    return last === null || last === tool.call.id;
  });

  const label = $derived(moved ? (pluginToolLabel(tool.call.name) ?? toolLabel(tool.call.name))
    : info?.label || (typeof args.label === "string" && args.label.trim()) || (typeof result.label === "string" && result.label) || m.chat_task_default_label());

  /** 没有登记时（旧对话、事件还没到）按工具本身的结果推断。 */
  const status = $derived.by<SubagentStatus>(() => {
    if (info) return info.status;
    if (tool.status === "failed") return "failed";
    if (result.status === "done") return "done";
    if (result.status === "failed") return "failed";
    return runActive || result.status === "running" || tool.status !== "completed" ? "running" : "done";
  });
  const summary = $derived(info?.summary ?? (typeof result.summary === "string" ? result.summary : ""));
  const firstSentence = $derived(summary.match(/^[\s\S]*?[。！？!?\n]/)?.[0]?.trim() ?? summary.trim());
  const steps = $derived(info?.steps ?? 0);

  const statusText = $derived.by(() => {
    if (!latest) return m.chat_task_status_superseded();
    switch (status) {
      case "running": return steps > 0 ? m.chat_task_status_running_steps({ steps }) : m.chat_task_status_running();
      case "waiting": return m.chat_task_status_waiting();
      case "done": return m.chat_task_status_done();
      case "failed": return m.chat_task_status_failed();
      case "stopped": return m.chat_task_status_stopped();
      case "interrupted": return m.chat_task_status_interrupted();
    }
  });
  const tone = $derived(!latest ? "" : status === "done" ? "text-success" : status === "failed" ? "text-destructive"
    : status === "waiting" ? "text-warning" : status === "running" ? "text-primary" : "");

  let open = $state(false);
  let busy = $state<"stop" | "continue" | SubagentAnswer | null>(null);
  let actionError = $state("");

  async function act(kind: "stop" | "continue" | SubagentAnswer) {
    if (busy || !conversationId || !name) return;
    busy = kind;
    actionError = "";
    try {
      if (kind === "stop") await agentSubagentStop(conversationId, name);
      else if (kind === "continue") await agentSubagentContinue(conversationId, name);
      else await agentSubagentAnswer(conversationId, name, kind);
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    } finally {
      busy = null;
    }
  }

  const processRunning = $derived(latest && (status === "running" || status === "waiting"));
</script>

<div class="my-1 min-w-0 overflow-hidden rounded-xl border border-hairline bg-card/60" data-testid="task-card"
  data-task-name={name} data-task-status={latest ? status : "superseded"} data-tool-id={tool.call.id}>
  <div class="flex min-h-9 min-w-0 items-center gap-1 py-1 pr-1.5 pl-3">
    <button type="button" class="flex min-w-0 flex-1 items-center gap-2 rounded-md py-0.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      aria-expanded={open} data-testid="task-card-toggle" onclick={() => { open = !open; }}>
      {#if latest && status === "running"}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-primary" />
      {:else if latest && status === "waiting"}<ShieldAlert class="size-3.5 shrink-0 text-warning" />
      {:else if latest && status === "done"}<Check class="size-3.5 shrink-0 text-success" />
      {:else if latest && status === "failed"}<CircleAlert class="size-3.5 shrink-0 text-destructive" />
      {:else if latest && (status === "stopped" || status === "interrupted")}<CirclePause class="size-3.5 shrink-0 text-muted-foreground" />
      {:else}<ClipboardList class="size-3.5 shrink-0 text-muted-foreground" />{/if}
      <span class="min-w-0 truncate font-medium text-foreground/90" data-testid="task-card-label">{label}</span>
      <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
      <span class="shrink-0 text-xs whitespace-nowrap {tone || 'text-muted-foreground'}" data-testid="task-card-status">{statusText}</span>
      <ChevronRight aria-hidden="true" class="size-3.5 shrink-0 text-muted-foreground transition-transform {open ? 'rotate-90' : ''}" />
    </button>
    {#if latest && (status === "running" || status === "waiting")}
      <Button size="sm" variant="ghost" class="h-7 shrink-0 gap-1 px-2 text-xs text-muted-foreground hover:text-destructive" data-testid="task-card-stop"
        disabled={!!busy} onclick={() => void act("stop")}><CircleStop class="size-3" />{m.chat_task_stop()}</Button>
    {:else if latest && status === "interrupted"}
      <Button size="sm" variant="ghost" class="h-7 shrink-0 gap-1 px-2 text-xs text-primary" data-testid="task-card-continue"
        disabled={!!busy} onclick={() => void act("continue")}><Play class="size-3" />{m.chat_task_continue()}</Button>
    {/if}
  </div>

  <!-- 转到后台的工具：结果是工具的原始数据，不摘一句；展开看工具卡片 -->
  {#if latest && !open && firstSentence && !moved && (status === "done" || status === "failed")}
    <p class="m-0 line-clamp-2 px-3 pb-2 text-[13px] leading-5 text-muted-foreground" data-testid="task-card-summary">{firstSentence}</p>
  {/if}

  {#if latest && status === "waiting"}
    <div class="border-t border-warning/40 bg-warning/5 px-3 pt-2.5 pb-3" data-testid="task-card-ask">
      <p class="m-0 text-[14px] leading-6 font-medium text-foreground">{m.chat_ask_confirm({ question: info?.question || label })}</p>
      <div class="mt-2.5 flex flex-wrap gap-2">
        <Button size="sm" data-testid="task-card-allow-once" disabled={!!busy} onclick={() => void act("allow-once")}>{m.chat_ask_allow_once()}</Button>
        <Button size="sm" variant="outline" data-testid="task-card-allow-conversation" disabled={!!busy} onclick={() => void act("allow-conversation")}>{m.chat_ask_allow_conversation()}</Button>
        <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10 hover:text-destructive" data-testid="task-card-deny" disabled={!!busy} onclick={() => void act("deny")}>{m.chat_ask_deny()}</Button>
      </div>
    </div>
  {/if}

  {#if actionError}<p role="alert" class="m-0 px-3 pb-2 text-xs text-destructive" data-testid="task-card-error">{actionError}</p>{/if}

  {#if open}
    <div class="max-h-96 min-w-0 space-y-2 overflow-auto border-t border-hairline bg-muted/30 px-3 py-2.5" data-testid="task-card-body">
      <TaskProcess {conversationId} {name} {info} {summary}
        showResult={latest && !moved && (status === "done" || status === "failed")} running={processRunning} />
    </div>
  {/if}
</div>
