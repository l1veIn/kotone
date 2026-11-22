<script lang="ts">
  // 行内节点：粗体、斜体、删除线、行内代码、链接、换行。HTML 与图片都按文字显示。
  import { openExternal } from "$lib/ipc";
  import { m } from "$lib/paraglide/messages.js";
  import { isSafeHttpUrl, tokenText, type Token, type Tokens } from "./markdown";
  import Self from "./MarkdownInline.svelte";

  let { tokens }: { tokens: Token[] } = $props();

  function open(event: MouseEvent, href: string) {
    event.preventDefault();
    void openExternal(href).catch(() => {});
  }
</script>

{#each tokens as token, index (index)}
  {#if token.type === "strong"}
    <strong class="font-semibold"><Self tokens={(token as Tokens.Strong).tokens} /></strong>
  {:else if token.type === "em"}
    <em><Self tokens={(token as Tokens.Em).tokens} /></em>
  {:else if token.type === "del"}
    <del><Self tokens={(token as Tokens.Del).tokens} /></del>
  {:else if token.type === "codespan"}
    <code class="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">{(token as Tokens.Codespan).text}</code>
  {:else if token.type === "br"}
    <br />
  {:else if token.type === "link"}
    {@const link = token as Tokens.Link}
    {#if isSafeHttpUrl(link.href)}
      <a href={link.href} title={link.href} class="text-primary underline underline-offset-2 hover:opacity-80"
        onclick={(event) => open(event, link.href)}><Self tokens={link.tokens} /></a>
    {:else}
      <Self tokens={link.tokens} />
    {/if}
  {:else if token.type === "image"}
    <span class="text-muted-foreground">{m.chat_markdown_image({ name: (token as Tokens.Image).text || m.chat_markdown_image_untitled() })}</span>
  {:else if token.type === "text" && (token as Tokens.Text).tokens?.length}
    <Self tokens={(token as Tokens.Text).tokens ?? []} />
  {:else if token.type === "checkbox"}
    <span aria-hidden="true">{(token as Tokens.Checkbox).checked ? "☑ " : "☐ "}</span>
  {:else}
    {tokenText(token)}
  {/if}
{/each}
