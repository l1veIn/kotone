<script lang="ts">
  // 程序里调的工具（ADR 046 D9）：编辑器里程序下面、列表详情里是同一块。“会调用：放歌（music）……”；要你允许的写出原因
  // （列表里带你允许的那份程序和“允许”按钮；编辑器里存的按钮变成“允许并保存”）；插件关着、调不了的写原因——照样能存，
  // 只是改好之前它不运行。
  import { Button } from "$lib/components/ui/button";
  import { toolLabel, toolReason, unusableTools, type ProgramTool } from "../../../../lib/automation";
  import { m } from "$lib/paraglide/messages.js";

  let { tools, error = null, editor = false, source = undefined, busy = false, onAllow = null }: {
    tools: ProgramTool[];
    /** 工具名不是直接写的（存不进来）。 */
    error?: string | null;
    /** 在编辑器里：不带“允许”按钮（存的按钮就是“允许并保存”），调不了的说一句“照样能存”。 */
    editor?: boolean;
    /** 列表里：你允许的那份程序（`automation_confirm` 核对的就是它）；undefined = 还在读。 */
    source?: string | null;
    busy?: boolean;
    onAllow?: (() => void) | null;
  } = $props();

  const waiting = $derived(tools.filter((t) => t.state === "confirm"));
  const unusable = $derived(unusableTools(tools));
  const tone: Record<ProgramTool["state"], string> = {
    ready: "border-hairline text-foreground/85",
    confirm: "border-warning/50 text-warning",
    off: "border-destructive/40 text-destructive",
    blocked: "border-destructive/40 text-destructive",
  };
</script>

{#if tools.length || error}
  <div class="flex flex-col gap-2 text-xs" data-testid="automation-tools">
    {#if tools.length}
      <p class="m-0 flex flex-wrap items-center gap-1.5 text-muted-foreground" data-testid="automation-tools-calls">
        <span>{m.auto_tools_calls()}</span>
        {#each tools as tool (tool.name)}
          <span class="rounded-md border px-1.5 py-px text-[11px] {tone[tool.state]}" title={[tool.title, tool.reason].filter(Boolean).join(" · ")}
            data-testid="automation-tool" data-state={tool.state}>{toolLabel(tool)}</span>
        {/each}
      </p>
    {/if}
    {#if waiting.length}
      <div class="rounded-lg border border-warning/40 bg-warning/8 px-3 py-2 leading-relaxed text-foreground" data-testid="automation-tools-confirm">
        <p class="m-0">{m.auto_tools_confirm_title()}</p>
        <ul class="m-0 mt-1 list-disc pl-4">
          {#each waiting as tool (tool.name)}<li>{toolReason(tool)}</li>{/each}
        </ul>
        <p class="m-0 mt-1 text-muted-foreground">{m.auto_tools_confirm_note()}</p>
        {#if !editor}
          {#if source === undefined}
            <p class="m-0 mt-2 text-muted-foreground">{m.auto_tools_source_loading()}</p>
          {:else if source !== null}
            <p class="m-0 mt-2 text-muted-foreground">{m.auto_tools_source_seen()}</p>
            <pre class="m-0 mt-1 max-h-40 overflow-auto rounded-md bg-background/60 px-2 py-1.5 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground/90"
              data-testid="automation-tools-source">{source}</pre>
            <div class="mt-2">
              <Button size="sm" disabled={busy} onclick={() => onAllow?.()} data-testid="automation-allow">{m.auto_tools_allow()}</Button>
            </div>
          {/if}
        {/if}
      </div>
    {/if}
    {#if unusable.length}
      <div class="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 leading-relaxed text-foreground" data-testid="automation-tools-unusable">
        <p class="m-0">{m.auto_tools_unusable_title()}{#if editor} {m.auto_tools_can_save()}{/if}</p>
        <ul class="m-0 mt-1 list-disc pl-4">
          {#each unusable as tool (tool.name)}<li>{toolReason(tool)}</li>{/each}
        </ul>
      </div>
    {/if}
    {#if error}<p class="m-0 text-destructive" data-testid="automation-tools-error">{error}</p>{/if}
  </div>
{/if}
