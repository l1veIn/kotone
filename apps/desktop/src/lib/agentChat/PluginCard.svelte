<script lang="ts">
  // 聊天里插件工具结果的卡片（ADR 038 第 2 步，照 MCP Apps 的“嵌在对话里”）：页面的小尺寸版本，
  // 是她查的那一刻的样子（只读）；点一下在右侧栏打开完整页面，旁边接着和她聊。
  // 插件做成 MCP App 的页面（ADR 039 D5）：卡片里是同一个 App（inline，进入可视区才加载），点头上一行打开完整页面；
  // App 里能点，所以卡片本身不整块当按钮。
  // 悬浮窗的小窗里（ADR 043 D6）没有右侧栏：给了 `onOpen` 就交给它（打开 Kotone、定位到这张卡片）。
  import { getContext } from "svelte";
  import { Maximize2 } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_MATCH } from "$plugins/lol/ui/types";
  import type { ToolDisplay } from "../agent";
  import PluginView from "../components/plugins/PluginView.svelte";
  import { FLOATING_PLUGIN_APP, viewOf } from "../pluginUi";
  import { openPluginPage } from "../stores/pluginPages";

  let { display, onOpen }: { display: ToolDisplay; onOpen?: () => void } = $props();
  const inFloating = getContext<boolean | undefined>(FLOATING_PLUGIN_APP);
  const uri = $derived(display.ui.uri);
  const floatingMatch = $derived(inFloating && uri === PAGE_MATCH);
  const found = $derived(viewOf(uri));
  const params = $derived((display.ui.params ?? {}) as Record<string, unknown>);

  function open() {
    if (onOpen) onOpen();
    else openPluginPage(uri, params, display.structuredContent ?? null);
  }
</script>

{#if found?.view.app}
  <div class={`my-1.5 max-w-[34rem] overflow-hidden ${floatingMatch ? "" : "rounded-xl border border-hairline bg-card/60"}`}
    class:floating-match={floatingMatch} data-testid="plugin-card" data-uri={uri} data-app="true">
    <button type="button" class="flex h-8 w-full items-center gap-2 border-b border-hairline px-3 text-left text-xs text-muted-foreground hover:text-foreground"
      data-testid="plugin-card-open" onclick={open}>
      <span class="min-w-0 flex-1 truncate font-medium text-foreground/85">{found.view.title()}</span>
      <span class="shrink-0">{m.chat_plugin_card_open()}</span>
      <Maximize2 class="size-3 shrink-0" />
    </button>
    <div data-testid="plugin-card-body">
      <PluginView {uri} {params} data={display.structuredContent ?? null} compact onFullscreen={open}
        onOpen={(next, nextParams) => (onOpen ? onOpen() : openPluginPage(next, nextParams ?? {}, null))} />
    </div>
  </div>
{:else if found?.view.card}
  <div class="my-1.5 max-w-[34rem] overflow-hidden rounded-xl border border-hairline bg-card/60 transition-colors hover:border-primary/40"
    data-testid="plugin-card" data-uri={uri}>
    <button type="button" class="flex h-8 w-full items-center gap-2 border-b border-hairline px-3 text-left text-xs text-muted-foreground hover:text-foreground"
      data-testid="plugin-card-open" onclick={open}>
      <span class="min-w-0 flex-1 truncate font-medium text-foreground/85">{found.view.title()}</span>
      <span class="shrink-0">{m.chat_plugin_card_open()}</span>
      <Maximize2 class="size-3 shrink-0" />
    </button>
    <!-- 卡片本身只读：点哪里都是打开完整页面 -->
    <div class="cursor-pointer px-3 py-2.5" role="button" tabindex="0" data-testid="plugin-card-body"
      onclick={open} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } }}>
      <PluginView {uri} {params} data={display.structuredContent ?? null} compact />
    </div>
  </div>
{/if}

<style>
  .floating-match > button {
    height: 28px;
    padding-inline: 6px;
    border-color: var(--floating-line, var(--hairline, var(--border)));
  }
  .floating-match > button:not(:hover) { color: var(--floating-muted, var(--muted-foreground)); }
  .floating-match > button > span:first-child { color: var(--floating-ink, var(--foreground)); }
</style>
