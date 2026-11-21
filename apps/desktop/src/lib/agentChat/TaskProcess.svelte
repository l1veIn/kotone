<!-- 交给后台的一件事展开后的内容：结果和一步步的过程（只读，复用工具卡片）。聊天里的卡片和右侧栏共用。 -->
<!-- 展开时读过程；还在做的时候每次状态变化（多一步、等确认、做完）都重新读。不显示推理过程。 -->
<script lang="ts">
  import { agentSubagentMessages, type AgentMessage, type SubagentInfo } from "../agent";
  import MessageMarkdown from "./markdown/MessageMarkdown.svelte";
  import { buildTurns, type ToolView } from "./timeline";
  import ToolCallCard from "./ToolCallCard.svelte";

  let { conversationId, name, info, summary = "", showResult = false, running = false }: {
    conversationId: string;
    name: string;
    info?: SubagentInfo;
    /** 做完 / 出错时的总结（showResult 时显示在过程上面）。 */
    summary?: string;
    showResult?: boolean;
    /** 还在做：过程里最后一步显示为进行中。 */
    running?: boolean;
  } = $props();

  let process = $state<AgentMessage[] | null>(null);
  let processError = $state("");
  let loading = $state(false);

  async function loadProcess() {
    if (!conversationId || !name) return;
    loading = true;
    try {
      process = await agentSubagentMessages(conversationId, name);
      processError = "";
    } catch (error) {
      processError = error instanceof Error ? error.message : String(error);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void info?.updatedAtMs;
    void info?.steps;
    void info?.status;
    void loadProcess();
  });

  /** 过程里的一行：交代（派给它的话）、它说的、它做的每一步、状态说明。 */
  type Row =
    | { kind: "task"; key: string; text: string }
    | { kind: "text"; key: string; text: string }
    | { kind: "tool"; key: string; tool: ToolView }
    | { kind: "notice"; key: string; text: string };
  const rows = $derived.by<Row[]>(() => {
    if (!process) return [];
    const out: Row[] = [];
    for (const turn of buildTurns(process, null, false)) {
      if (turn.user) {
        const text = turn.user.text.replace(/<player-request>[\s\S]*?<\/player-request>\s*/g, "").trim();
        if (text) out.push({ kind: "task", key: `u${turn.id}`, text });
      }
      for (const block of turn.blocks) {
        if (block.kind === "text") out.push({ kind: "text", key: block.key, text: block.text });
        else if (block.kind === "tool") out.push({ kind: "tool", key: block.key, tool: block.tool });
        else if (block.kind === "notice") out.push({ kind: "notice", key: block.key, text: block.text });
      }
    }
    return out;
  });
</script>

{#if showResult && summary}
  <section aria-label="结果">
    <h4 class="mt-0 mb-1 text-[11px] font-medium text-muted-foreground">结果</h4>
    <p class="m-0 text-[13px] leading-5 whitespace-pre-wrap text-foreground/90" data-testid="task-card-summary-full">{summary}</p>
  </section>
{/if}
<section aria-label="过程" data-testid="task-card-process">
  <h4 class="mt-0 mb-1 text-[11px] font-medium text-muted-foreground">过程</h4>
  {#if processError}
    <p class="m-0 text-xs text-destructive">读不到过程：{processError}</p>
  {:else if process === null}
    <p class="m-0 text-xs text-muted-foreground">{loading ? "读取中…" : ""}</p>
  {:else if rows.length === 0}
    <p class="m-0 text-xs text-muted-foreground">还没有开始</p>
  {:else}
    <div class="min-w-0 space-y-1">
      {#each rows as row (row.key)}
        {#if row.kind === "task"}
          <p class="m-0 line-clamp-3 text-xs leading-5 text-muted-foreground" data-testid="task-card-step-task">交代：{row.text}</p>
        {:else if row.kind === "text"}
          <div class="text-[13px]" data-testid="task-card-step-text"><MessageMarkdown text={row.text} /></div>
        {:else if row.kind === "tool"}
          <ToolCallCard tool={row.tool} runActive={running} />
        {:else}
          <p class="m-0 text-xs text-muted-foreground">{row.text}</p>
        {/if}
      {/each}
    </div>
  {/if}
</section>
