<script lang="ts">
  // 块级节点：段落、标题、列表、引用、代码块、表格、分隔线。
  import { tokenText, type Token, type Tokens } from "./markdown";
  import Inline from "./MarkdownInline.svelte";
  import Self from "./MarkdownBlocks.svelte";

  let { tokens }: { tokens: Token[] } = $props();
  const headingSize: Record<number, string> = { 1: "text-base", 2: "text-[0.95rem]", 3: "text-sm" };
</script>

{#each tokens as token, index (index)}
  {#if token.type === "paragraph"}
    <p class="my-1.5"><Inline tokens={(token as Tokens.Paragraph).tokens} /></p>
  {:else if token.type === "heading"}
    {@const heading = token as Tokens.Heading}
    <p class="mt-3 mb-1.5 font-semibold {headingSize[heading.depth] ?? 'text-sm'}" role="heading" aria-level={heading.depth}><Inline tokens={heading.tokens} /></p>
  {:else if token.type === "code"}
    <pre class="my-2 overflow-x-auto rounded-lg border bg-muted/50 p-3 font-mono text-xs leading-5"><code>{(token as Tokens.Code).text}</code></pre>
  {:else if token.type === "blockquote"}
    <blockquote class="my-2 border-l-2 border-primary/40 pl-3 text-muted-foreground"><Self tokens={(token as Tokens.Blockquote).tokens} /></blockquote>
  {:else if token.type === "list"}
    {@const list = token as Tokens.List}
    {#if list.ordered}
      <ol class="my-1.5 list-decimal space-y-0.5 pl-5" start={typeof list.start === "number" ? list.start : undefined}>
        {#each list.items as item, i (i)}<li><Self tokens={item.tokens} /></li>{/each}
      </ol>
    {:else}
      <ul class="my-1.5 space-y-0.5 pl-5 {list.items.some((item) => item.task) ? 'list-none' : 'list-disc'}">
        {#each list.items as item, i (i)}<li><Self tokens={item.tokens} /></li>{/each}
      </ul>
    {/if}
  {:else if token.type === "table"}
    {@const table = token as Tokens.Table}
    <div class="my-2 overflow-x-auto">
      <table class="w-full border-collapse text-xs">
        <thead><tr>{#each table.header as cell, i (i)}<th class="border px-2 py-1 text-left font-semibold" style:text-align={table.align[i] ?? undefined}><Inline tokens={cell.tokens} /></th>{/each}</tr></thead>
        <tbody>
          {#each table.rows as row, r (r)}
            <tr>{#each row as cell, i (i)}<td class="border px-2 py-1" style:text-align={table.align[i] ?? undefined}><Inline tokens={cell.tokens} /></td>{/each}</tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if token.type === "hr"}
    <hr class="my-3 border-border" />
  {:else if token.type === "space" || token.type === "def"}
    <!-- 空行与链接定义不显示 -->
  {:else if token.type === "text"}
    {@const text = token as Tokens.Text}
    <!-- 紧凑列表项里的文字：直接按行内渲染 -->
    {#if text.tokens?.length}<Inline tokens={text.tokens} />{:else}{text.text}{/if}
  {:else if token.type === "checkbox"}
    <Inline tokens={[token]} />
  {:else}
    <!-- html 等：原样显示成文字 -->
    <p class="my-1.5 whitespace-pre-wrap">{tokenText(token)}</p>
  {/if}
{/each}
