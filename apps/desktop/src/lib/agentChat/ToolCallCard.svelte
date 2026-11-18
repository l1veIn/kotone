<!-- Svelte presentation adapted from Codeg chat tool-call components at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/ToolCallCard.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: one-line flow row with an expandable body and an approval card, laid out after -->
<!-- DeepSeek Harness (GenericCommandCard / ApprovalPanel, MIT); Chinese tool labels; the permission -->
<!-- buttons (允许 / 本段对话都允许 / 拒绝) for kotone://agent-ask are Kotone's own. -->
<script lang="ts">
  import { Check, ChevronRight, Circle, CircleAlert, LoaderCircle, ShieldAlert, Wrench, Zap } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { toolLabel, type AgentAsk, type Answer } from "../agent";
  import type { ToolView } from "./timeline";
  import { formatToolJson, toolPresentation } from "./toolPresentation";
  import { changedLocations, openLocation } from "../stores/navigation";

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
  /** 改了设置的工具：“改了 xxx → 去看看”，点了跳到设置里的位置。 */
  const changed = $derived(tool.status === "completed" ? changedLocations(tool.call.name, tool.content, tool.call.arguments) : []);

  async function answer(value: Answer) {
    if (!ask || !onAnswer || answering) return;
    answering = value;
    answerError = "";
    try { await onAnswer(ask.askId, value); }
    catch (error) { answerError = error instanceof Error ? error.message : String(error); }
    finally { answering = null; }
  }
</script>

<div class="min-w-0" data-testid="agent-tool-card" data-tool-id={tool.call.id} data-tool-name={tool.call.name}
  data-tool-status={presentation.incomplete ? "incomplete" : tool.status}>
  <details bind:open class="group/tool min-w-0">
    <summary class="flex h-7 w-fit max-w-full cursor-pointer list-none items-center gap-2 rounded-md pr-1.5 text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
      {#if saved}<Zap class="size-3.5 shrink-0" />{:else}<Wrench class="size-3.5 shrink-0" />{/if}
      <span class="min-w-0 truncate text-foreground/85" title={tool.call.name}>{title}</span>
      <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
      <span class="flex shrink-0 items-center gap-1 text-xs whitespace-nowrap {presentation.failed ? 'text-destructive' : tool.status === 'completed' ? 'text-success' : presentation.spinning ? 'text-primary' : ''}" data-testid="agent-tool-status">
        {#if presentation.spinning}<LoaderCircle class="size-3 animate-spin" />
        {:else if tool.status === "completed"}<Check class="size-3" />
        {:else if presentation.failed || presentation.incomplete}<CircleAlert class="size-3" />
        {:else}<Circle class="size-3" />{/if}
        {presentation.label}
      </span>
      <ChevronRight aria-hidden="true" class="size-3.5 shrink-0 opacity-0 transition-[opacity,transform] group-hover/tool:opacity-100 group-open/tool:rotate-90 group-open/tool:opacity-100" />
    </summary>
    {#if open}
      <div class="mt-1 mb-2 ml-1 max-h-72 min-w-0 space-y-3 overflow-auto rounded-xl border border-hairline bg-muted/40 px-4 py-3 text-xs">
        <p class="m-0 font-mono text-[11px] break-all text-muted-foreground">{tool.call.name}</p>
        {#if presentation.incomplete}<p class="m-0 text-muted-foreground">这一轮结束前，这个工具没有返回结果。</p>{/if}
        {#if input && input !== "{}"}
          <section aria-label="参数"><h4 class="mt-0 mb-1 text-[11px] font-medium text-muted-foreground">参数</h4><pre class="m-0 font-mono text-xs leading-5 break-words whitespace-pre-wrap text-foreground/90">{input}</pre></section>
        {/if}
        {#if output}
          <section aria-label="结果">
            <h4 class="mt-0 mb-1 text-[11px] font-medium text-muted-foreground">结果{#if tool.untrusted}<span class="ml-1 font-normal">（来自屏幕或游戏聊天，仅供参考）</span>{/if}</h4>
            <pre class="m-0 font-mono text-xs leading-5 break-words whitespace-pre-wrap {presentation.failed ? 'text-destructive' : 'text-foreground/90'}">{output}</pre>
          </section>
        {/if}
      </div>
    {/if}
  </details>
  {#if changed.length}
    <div class="mb-1 ml-5 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground" data-testid="agent-tool-changed">
      <span>改了</span>
      {#each changed as item (item.label + item.page)}
        <button type="button" class="rounded-md bg-primary/10 px-1.5 py-0.5 text-primary hover:bg-primary/20" title={`去看看：${item.page}`}
          data-testid="agent-tool-changed-link" data-page={item.page} onclick={() => openLocation(item.page)}>{item.label} →</button>
      {/each}
    </div>
  {/if}

  {#if ask}
    <div class="my-2 overflow-hidden rounded-2xl border border-warning/45 bg-composer shadow-lg" data-testid="agent-ask" data-ask-id={ask.askId}>
      <div class="flex items-center gap-2 bg-warning/10 px-4 py-2 text-xs font-medium text-warning">
        <ShieldAlert class="size-3.5" />请求权限
      </div>
      <div class="px-4 pt-3 pb-3.5">
        <p class="m-0 text-[15px] leading-6 font-medium text-foreground">需要你确认：{ask.request.title || title}</p>
        {#if ask.request.reason}<p class="m-0 mt-0.5 text-[13px] leading-5 text-muted-foreground">{ask.request.reason}</p>{/if}
        <div class="mt-3 flex flex-wrap gap-2">
          <Button size="sm" data-testid="agent-ask-allow-once" disabled={!!answering} onclick={() => void answer("allow-once")}>允许</Button>
          <Button size="sm" variant="outline" data-testid="agent-ask-allow-conversation" disabled={!!answering} onclick={() => void answer("allow-for-conversation")}>本段对话都允许</Button>
          <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10 hover:text-destructive" data-testid="agent-ask-deny" disabled={!!answering} onclick={() => void answer("deny")}>拒绝</Button>
        </div>
        {#if answerError}<p role="alert" class="mt-2 mb-0 text-xs text-destructive">{answerError}</p>{/if}
      </div>
    </div>
  {/if}
</div>
