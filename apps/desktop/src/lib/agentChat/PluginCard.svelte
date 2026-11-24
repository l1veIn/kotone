<script lang="ts">
  // 聊天里插件工具结果的卡片（ADR 038 第 2 步，照 MCP Apps 的“嵌在对话里”）：页面的小尺寸版本，
  // 是她查的那一刻的样子（只读）；点一下在右侧栏打开完整页面，旁边接着和她聊。
  import { Maximize2 } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import type { ToolDisplay } from "../agent";
  import PluginView from "../components/plugins/PluginView.svelte";
  import { viewOf } from "../pluginUi";
  import { openPluginPage } from "../stores/pluginPages";

  let { display }: { display: ToolDisplay } = $props();
  const uri = $derived(display.ui.uri);
  const found = $derived(viewOf(uri));
  const params = $derived((display.ui.params ?? {}) as Record<string, unknown>);

  function open() {
    openPluginPage(uri, params, display.structuredContent ?? null);
  }
</script>

{#if found?.view.card}
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
