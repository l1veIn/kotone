<script lang="ts">
  // 第三方插件点开以后（MCP 方案第 2 期，和内置插件同一个容器 PluginPage.svelte）：
  // - 它有 MCP App（`ui://` 页面）：整块放它的 App（McpAppFrame，fullscreen；页面从它的服务器取，CSP 用它声明的，
  //   宿主按 iframe 绑定这个插件），有好几页时上面一排切换；
  // - 没有：一页工具列表（名字、说明、标没标只读、给谁）。
  // 上面一条是它的状态（连上没有、没连上的原因、缺的 Key、还没信任），连接、Key、信任、删除在顶栏的“设置”里。
  import { RefreshCw } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import McpAppFrame from "./McpAppFrame.svelte";
  import PluginLogo from "./PluginLogo.svelte";
  import { reconnectServer, remoteIcon, remoteOf, remoteTitle } from "$lib/mcpServers";
  import { pluginEnabled } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin, openSettings }: PluginPageProps = $props();
  const remote = $derived(remoteOf(plugin));
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const id = $derived(plugin.manifest.id);
  const missingKeys = $derived((remote?.keys ?? []).filter((k) => !k.filled).map((k) => k.name));
  let pageUri = $state<string | null>(null);
  const pages = $derived(remote?.pages ?? []);
  const shown = $derived(pages.find((p) => p.uri === pageUri)?.uri ?? pages[0]?.uri ?? null);
  let reconnecting = $state(false);

  async function reconnect() {
    reconnecting = true;
    try {
      const error = await reconnectServer(id);
      toast(!error, error ? m.system_mcp_reconnect_failed({ error }) : m.system_mcp_reconnected());
    } catch (error) {
      toast(false, m.system_mcp_reconnect_failed({ error: errText(error) }));
    } finally {
      reconnecting = false;
    }
  }

  const statusText = (status: string) =>
    ({
      connected: m.system_mcp_status_connected(),
      connecting: m.system_mcp_status_connecting(),
      failed: m.system_mcp_status_failed(),
    })[status] ?? m.system_mcp_status_not_connected();
</script>

<div class="flex min-h-full flex-col" data-testid="remote-plugin-page" data-plugin-id={id} data-status={remote?.status}>
  <div class="flex flex-wrap items-center gap-2 border-b border-hairline px-4 py-2 text-xs">
    <PluginLogo {id} name={id} src={remoteIcon(plugin)} size={20} off={!on} testid="remote-plugin-logo" />
    {#if remoteTitle(plugin)}<span class="text-muted-foreground">{m.system_mcp_server_says({ title: remoteTitle(plugin) ?? "" })}</span>{/if}
    <span class="rounded bg-muted px-1.5 py-0.5 text-muted-foreground" data-testid="remote-plugin-status">{statusText(remote?.status ?? "")}</span>
    {#if !remote?.trusted}<span class="rounded bg-warning/15 px-1.5 py-0.5 text-warning" data-testid="remote-plugin-untrusted">{m.system_mcp_untrusted()}</span>{/if}
    {#if missingKeys.length}<span class="rounded bg-warning/15 px-1.5 py-0.5 text-warning" data-testid="remote-plugin-needs-key">{m.system_mcp_needs_key()}: {missingKeys.join(", ")}</span>{/if}
    <span class="ml-auto flex items-center gap-1.5">
      {#if on}
        <Button size="sm" variant="ghost" class="h-7 gap-1 text-xs" disabled={reconnecting} data-testid="remote-plugin-reconnect" onclick={() => void reconnect()}>
          <RefreshCw class="size-3.5 {reconnecting ? 'animate-spin' : ''}" />{m.system_mcp_reconnect()}
        </Button>
      {/if}
      <!-- 有要去设置里做的事（缺 Key、没连上）时多一个直达的按钮；平时用顶栏的“设置” -->
      {#if openSettings && (missingKeys.length || remote?.status === "failed")}<Button size="sm" variant="outline" class="h-7 text-xs" data-testid="remote-plugin-open-settings" onclick={openSettings}>{m.system_mcp_open_settings()}</Button>{/if}
    </span>
  </div>
  {#if remote?.status === "failed" && remote.error}
    <p class="m-0 border-b border-hairline bg-destructive/5 px-4 py-2 text-xs text-destructive" data-testid="remote-plugin-error">{remote.error}</p>
  {/if}

  {#if !on}
    <p class="m-0 px-6 py-8 text-sm text-muted-foreground" data-testid="remote-plugin-off">{m.system_mcp_off_note()}</p>
  {:else if shown}
    {#if pages.length > 1}
      <div class="flex flex-wrap gap-1 px-4 pt-2 text-xs" role="tablist">
        {#each pages as page (page.uri)}
          <button type="button" role="tab" aria-selected={page.uri === shown}
            class="rounded-md px-2 py-1 {page.uri === shown ? 'bg-muted font-medium text-foreground' : 'text-muted-foreground hover:text-foreground'}"
            onclick={() => (pageUri = page.uri)}>{page.title}</button>
        {/each}
      </div>
    {/if}
    {#key shown}
      <div class="min-h-0 flex-1" data-testid="remote-plugin-app" data-uri={shown}>
        <McpAppFrame uri={shown} pluginId={id} displayMode="fullscreen" />
      </div>
    {/key}
  {:else}
    <div class="mx-auto w-full max-w-3xl px-6 py-6" data-testid="remote-plugin-tools">
      <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_tools_title()}</h2>
      <p class="mt-1 mb-4 text-xs text-muted-foreground">{m.system_mcp_tools_hint({ companion: $companionName })}</p>
      {#if remote?.toolList?.length}
        <ul class="m-0 list-none divide-y divide-hairline rounded-xl border border-hairline p-0">
          {#each remote.toolList as tool (tool.name)}
            <li class="px-4 py-2.5" data-testid="remote-plugin-tool" data-tool={tool.name}>
              <p class="m-0 flex flex-wrap items-center gap-2 text-sm">
                <code class="font-mono text-xs text-foreground">{tool.name}</code>
                {#if tool.title}<span class="text-xs text-muted-foreground">{tool.title}</span>{/if}
                {#if tool.readOnly}<span class="rounded bg-muted px-1.5 text-[10px] text-muted-foreground" data-testid="remote-plugin-tool-read-only">{m.system_mcp_tool_read_only()}</span>{/if}
                {#if !tool.visibility.includes("model")}<span class="rounded bg-muted px-1.5 text-[10px] text-muted-foreground">{m.system_mcp_tool_app_only()}</span>{/if}
              </p>
              {#if tool.description}<p class="m-0 mt-0.5 text-xs leading-relaxed text-muted-foreground">{tool.description}</p>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="m-0 text-xs text-muted-foreground" data-testid="remote-plugin-no-tools">{m.system_mcp_no_tools()}</p>
      {/if}
    </div>
  {/if}
</div>
