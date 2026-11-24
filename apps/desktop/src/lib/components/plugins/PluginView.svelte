<script lang="ts">
  // 显示一个插件页面（ADR 038，照 MCP Apps：宿主按 `ui://` 地址找到页面，交给它数据和一座桥）。
  // compact = 聊天里的卡片（只读快照）；否则是完整页面。没登记的地址说一句打不开。
  // 插件做成 MCP App 的页面（ADR 039 D5）放进 McpAppFrame：卡片是 inline，完整页面是 fullscreen；App 要开本插件的
  // 另一个页面时照旧交给 onOpen，卡片里的 App 请求全屏时交给 onFullscreen（聊天卡片：在右侧栏打开）。
  import { pluginCall } from "$lib/plugins";
  import { viewOf, type PluginBridge } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import McpAppFrame from "./McpAppFrame.svelte";

  let { uri, params = {}, data = null, compact = false, onOpen, onReplace, onFullscreen }: {
    uri: string;
    params?: Record<string, unknown>;
    data?: unknown;
    compact?: boolean;
    /** 页面里点进下一页（右侧栏、插件页各自决定放在哪）。 */
    onOpen?: (uri: string, params?: Record<string, unknown>, data?: unknown) => void;
    /** 页面记下自己现在的样子（返回到它时用）。 */
    onReplace?: (params: Record<string, unknown>, data: unknown) => void;
    /** 卡片里的 App 请求全屏。 */
    onFullscreen?: () => void;
  } = $props();

  const found = $derived(viewOf(uri));
  const View = $derived(found ? (compact ? found.view.card : found.view.page) : undefined);
  const bridge: PluginBridge = {
    call: (tool, args = {}) => {
      const plugin = viewOf(uri)?.plugin;
      if (!plugin) return Promise.reject(new Error(uri));
      return pluginCall(plugin, tool, args);
    },
    openPage: (next, nextParams, nextData) => onOpen?.(next, nextParams, nextData),
    replace: (nextParams, nextData) => onReplace?.(nextParams, nextData),
  };
</script>

{#if found?.view.app}
  <McpAppFrame {uri} {params} {data} displayMode={compact ? "inline" : "fullscreen"}
    onOpenPage={(next, nextParams) => onOpen?.(next, nextParams)} onRequestFullscreen={onFullscreen} />
{:else if View}
  <View {uri} {params} {data} {bridge} />
{:else if !compact}
  <p class="m-0 px-1 py-6 text-center text-xs text-muted-foreground" data-testid="plugin-view-missing">{m.chat_plugin_page_missing()}</p>
{/if}
