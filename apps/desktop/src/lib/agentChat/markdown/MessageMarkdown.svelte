<script lang="ts">
  // 琴音回答的 Markdown（流式时每个片段到来都重新解析；内容通常很短）。
  import { lexMarkdown } from "./markdown";
  import MarkdownBlocks from "./MarkdownBlocks.svelte";

  let { text }: { text: string } = $props();
  const tokens = $derived(lexMarkdown(text));
</script>

<div class="min-w-0 text-sm leading-relaxed break-words [&>*:first-child]:mt-0 [&>*:last-child]:mb-0" data-markdown>
  <MarkdownBlocks {tokens} />
</div>
