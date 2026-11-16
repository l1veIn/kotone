<!-- Svelte presentation adapted from Codeg chat tool-call components at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/ToolCallCard.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: Chinese tool labels, collapsed arguments / result, and the permission buttons -->
<!-- (允许 / 本段对话都允许 / 拒绝) for kotone://agent-ask, which are Kotone's own. -->
<script lang="ts">
  import { Check, ChevronRight, Circle, CircleAlert, LoaderCircle, ShieldQuestion, Wrench, Zap } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { toolLabel, type AgentAsk, type Answer } from "../agent";
  import type { ToolView } from "./timeline";
  import { formatToolJson, toolPresentation } from "./toolPresentation";

  let { tool, runActive = false, ask, onAnswer }: {
    tool: ToolView;
    runActive?: boolean;
    ask?: AgentAsk;
    onAnswer?: (askId: string, answer: Answer) => Promise<void>;
  } = $props();

  let open = $state(false);
  let answering = $state<Answer | null>(null);
  let answerError = $state("");
  const presentation = $derived(toolPresentation(tool.status, runActive));
  const title = $derived(toolLabel(tool.call.name));
  const saved = $derived(tool.call.name.startsWith("saved."));
  const input = $derived(open ? formatToolJson(tool.call.arguments) : "");
  const output = $derived(open ? formatToolJson(tool.content) : "");

  async function answer(value: Answer) {
    if (!ask || !onAnswer || answering) return;
    answering = value;
    answerError = "";
    try { await onAnswer(ask.askId, value); }
    catch (error) { answerError = error instanceof Error ? error.message : String(error); }
    finally { answering = null; }
  }
</script>

<div class="min-w-0 rounded-lg border bg-card/40 px-2.5 py-1.5 {ask ? 'border-warning/60 bg-warning/5' : ''}"
  data-testid="agent-tool-card" data-tool-id={tool.call.id} data-tool-name={tool.call.name} data-tool-status={presentation.incomplete ? "incomplete" : tool.status}>
  <details bind:open class="group/tool min-w-0">
    <summary class="flex max-w-full cursor-pointer list-none items-center gap-2 rounded-sm text-xs leading-5 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
      {#if saved}<Zap class="size-3.5 shrink-0" />{:else}<Wrench class="size-3.5 shrink-0" />{/if}
      <span class="min-w-0 truncate font-medium text-foreground/90" title={tool.call.name}>{title}</span>
      <span class="flex shrink-0 items-center gap-1 text-[11px] whitespace-nowrap {presentation.failed ? 'text-destructive' : tool.status === 'completed' ? 'text-success' : ''}" data-testid="agent-tool-status">
        {#if presentation.spinning}<LoaderCircle class="size-3 animate-spin" />
        {:else if tool.status === "completed"}<Check class="size-3" />
        {:else if presentation.failed || presentation.incomplete}<CircleAlert class="size-3" />
        {:else}<Circle class="size-3" />{/if}
        {presentation.label}
      </span>
      <ChevronRight aria-hidden="true" class="ml-auto size-3 shrink-0 transition-transform group-open/tool:rotate-90" />
    </summary>
    {#if open}
      <div class="mt-2 mb-1 max-h-72 min-w-0 space-y-2.5 overflow-auto rounded-md bg-muted/40 p-2.5 text-xs">
        <p class="m-0 font-mono text-[11px] break-all text-muted-foreground">{tool.call.name}</p>
        {#if presentation.incomplete}<p class="m-0 text-muted-foreground">这一轮结束前，这个工具没有返回结果。</p>{/if}
        {#if input && input !== "{}"}
          <section aria-label="参数"><h4 class="mt-0 mb-1 text-[10px] font-medium text-muted-foreground">参数</h4><pre class="m-0 font-mono text-[11px] leading-5 break-words whitespace-pre-wrap">{input}</pre></section>
        {/if}
        {#if output}
          <section aria-label="结果">
            <h4 class="mt-0 mb-1 text-[10px] font-medium text-muted-foreground">结果{#if tool.untrusted}<span class="ml-1 font-normal">（来自屏幕或游戏聊天，仅供参考）</span>{/if}</h4>
            <pre class="m-0 font-mono text-[11px] leading-5 break-words whitespace-pre-wrap">{output}</pre>
          </section>
        {/if}
      </div>
    {/if}
  </details>

  {#if ask}
    <div class="mt-1.5 border-t border-warning/30 pt-2" data-testid="agent-ask" data-ask-id={ask.askId}>
      <p class="m-0 flex items-start gap-1.5 text-xs text-foreground">
        <ShieldQuestion class="mt-0.5 size-3.5 shrink-0 text-warning" />
        <span><strong class="font-semibold">需要你确认：{ask.request.title || title}</strong>{#if ask.request.reason}<span class="text-muted-foreground"> · {ask.request.reason}</span>{/if}</span>
      </p>
      <div class="mt-2 flex flex-wrap gap-1.5">
        <Button size="sm" data-testid="agent-ask-allow-once" disabled={!!answering} onclick={() => void answer("allow-once")}>允许</Button>
        <Button size="sm" variant="secondary" data-testid="agent-ask-allow-conversation" disabled={!!answering} onclick={() => void answer("allow-for-conversation")}>本段对话都允许</Button>
        <Button size="sm" variant="destructive" data-testid="agent-ask-deny" disabled={!!answering} onclick={() => void answer("deny")}>拒绝</Button>
      </div>
      {#if answerError}<p role="alert" class="mt-1.5 mb-0 text-xs text-destructive">{answerError}</p>{/if}
    </div>
  {/if}
</div>
