<script lang="ts">
  // 左侧栏的「插件」（ADR 038，2026-11-24 玩家定的）：像应用启动器，每个插件一张卡片，对应 MCP Apps 里的一个 App。
  // 卡片上有插件自己的 logo（ADR 039 D8，没有 logo 的用首字块）、名字、一句话、开关（关着的卡片变淡）；
  // 点卡片（不是开关）进这个插件的 App，App 占满内容区、用它自己的界面。插件自己的数据开关和风险提示在它自己的 App 里。
  // 第三方插件（MCP 方案第 2 期：玩家自己加的别人的 MCP 服务器）在下面另一组：logo 用服务器声明的图标（没有就是首字块），
  // 卡片上标“第三方”和状态（没连上、还没填 Key、还没信任）；右上角“加 MCP 服务器”（粘贴配置、从 Claude / VS Code 导入，
  // 加之前有信任提示）。点开和内置的进同一个位置（PluginPage.svelte：它的 MCP App，或一页工具列表）。
  import { onMount } from "svelte";
  import { Plus } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { listPlugins, pluginDescription, pluginEnabled, pluginName, pluginsStore, setPluginEnabled, type PluginInfo } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import { isRemote, remoteIcon, remoteOf, remoteTitle } from "$lib/mcpServers";
  import PluginLogo from "$lib/components/plugins/PluginLogo.svelte";
  import AddMcpServerDialog from "$lib/components/plugins/AddMcpServerDialog.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let adding = $state(false);
  const builtins = $derived($pluginsStore.filter((p) => !isRemote(p)));
  const remotes = $derived($pluginsStore.filter((p) => isRemote(p)));

  /** 刚加了第三方插件：这之前一直隔一会儿重新读清单（它在后台连，卡片上的状态跟着变）。 */
  let pollUntil = 0;
  onMount(() => {
    // 每次进来都读一次：第三方插件的连接状态会变（内置的清单不变，读一次也很便宜）
    void listPlugins().catch((error) => toast(false, errText(error)));
    const timer = setInterval(() => {
      const busy = remotes.some((p) => remoteOf(p)?.status === "connecting");
      if (busy || Date.now() < pollUntil) void listPlugins().catch(() => {});
    }, 2000);
    return () => clearInterval(timer);
  });

  async function toggle(id: string, value: boolean) {
    try { await setPluginEnabled(id, value); } catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
  }

  /** 第三方插件卡片上的一句状态（没有要说的 = null）。 */
  function remoteStatus(plugin: PluginInfo, on: boolean): { text: string; warn: boolean } | null {
    const remote = remoteOf(plugin);
    if (!remote || !on) return null;
    if (remote.keys.some((k) => !k.filled)) return { text: m.system_mcp_needs_key(), warn: true };
    if (remote.status === "failed") return { text: m.system_mcp_status_failed(), warn: true };
    if (!remote.trusted) return { text: m.system_mcp_untrusted(), warn: true };
    if (remote.status === "connected") return { text: m.system_mcp_status_connected(), warn: false };
    if (remote.status === "connecting") return { text: m.system_mcp_status_connecting(), warn: false };
    return { text: m.system_mcp_status_not_connected(), warn: false };
  }
</script>

{#snippet card(plugin: PluginInfo)}
  {@const manifest = plugin.manifest}
  {@const ui = PLUGIN_UI[manifest.id]}
  {@const on = pluginEnabled($settingsStore, manifest)}
  {@const name = pluginName(manifest)}
  {@const remote = isRemote(plugin)}
  {@const status = remoteStatus(plugin, on)}
  {@const subtitle = remote ? remoteTitle(plugin) : null}
  <div class="group relative flex min-h-36 flex-col rounded-2xl border border-hairline bg-card p-4 transition-[border-color,box-shadow,opacity]
      hover:border-primary/45 hover:shadow-sm {on ? '' : 'opacity-60 hover:opacity-90'}"
    data-testid={`plugins-card-${manifest.id}`} data-enabled={on} data-remote={remote}>
    <!-- 整张卡片可点（开关除外）：打开这个插件的 App -->
    <button type="button" class="absolute inset-0 rounded-2xl focus-visible:outline-2 focus-visible:outline-primary disabled:cursor-default"
      aria-label={m.shell_plugins_open_aria({ name })} data-testid={`plugins-card-${manifest.id}-open`} disabled={!ui && !remote}
      onclick={() => openTarget({ page: "plugin", id: manifest.id })}></button>
    <div class="pointer-events-none relative flex items-start justify-between gap-3">
      <!-- 插件自己的 logo（ADR 039 D8）：自带底色的方块，亮暗主题一样；关着的变灰。第三方的用服务器声明的图标 -->
      <PluginLogo id={manifest.id} {name} src={remote ? remoteIcon(plugin) : null} size={40} off={!on} testid={`plugins-card-${manifest.id}-logo`} />
      <div class="pointer-events-auto relative z-10">
        <!-- 受控：显示的总是设置里的值，没存上时不会停在错误的位置 -->
        <Switch bind:checked={() => on, (next) => void toggle(manifest.id, next)}
          aria-label={m.system_plugins_enable({ name })} data-testid={`plugins-card-${manifest.id}-enabled`} />
      </div>
    </div>
    <p class="pointer-events-none relative mt-3 mb-0 flex flex-wrap items-center gap-2 text-sm font-semibold text-foreground">
      {name}
      {#if remote}<span class="rounded bg-muted px-1.5 text-[10px] font-normal text-muted-foreground" data-testid={`plugins-card-${manifest.id}-third-party`}>{m.system_mcp_badge()}</span>{/if}
      {#if !on}<span class="rounded bg-muted px-1.5 text-[10px] font-normal text-muted-foreground" data-testid={`plugins-card-${manifest.id}-off`}>{m.system_plugins_off_badge()}</span>{/if}
    </p>
    {#if subtitle}<p class="pointer-events-none relative mt-0.5 mb-0 truncate text-[11px] text-muted-foreground">{subtitle}</p>{/if}
    <p class="pointer-events-none relative mt-1 mb-0 line-clamp-3 text-xs leading-relaxed text-muted-foreground {remote ? 'font-mono text-[11px] break-all' : ''}">
      {remote ? (remoteOf(plugin)?.server?.description || pluginDescription(manifest)) : pluginDescription(manifest)}
    </p>
    {#if status}
      <p class="pointer-events-none relative mt-auto mb-0 pt-2 text-[11px] {status.warn ? 'text-warning' : 'text-muted-foreground'}"
        data-testid={`plugins-card-${manifest.id}-status`}>{status.text}</p>
    {/if}
  </div>
{/snippet}

<div class="mx-auto w-full max-w-5xl px-8 pt-7 pb-4" data-testid="plugins-page">
  <div class="flex items-start justify-between gap-4">
    <h1 class="m-0 text-xl font-semibold tracking-tight text-foreground">{m.shell_sidebar_plugins()}</h1>
    <Button size="sm" variant="outline" class="gap-1.5" data-testid="plugins-add-mcp" onclick={() => (adding = true)}><Plus class="size-3.5" />{m.system_mcp_add()}</Button>
  </div>
  <p class="mt-1.5 mb-0 max-w-3xl text-xs leading-relaxed text-muted-foreground">{m.system_plugins_description({ companion: $companionName })}</p>

  <div class="mt-6 grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(15rem,1fr))]" data-testid="plugins-grid">
    {#each builtins as plugin (plugin.manifest.id)}
      {@render card(plugin)}
    {/each}
  </div>

  {#if remotes.length}
    <h2 class="mt-8 mb-0 text-sm font-semibold text-foreground">{m.system_mcp_section()}</h2>
    <p class="mt-1 mb-0 max-w-3xl text-xs leading-relaxed text-muted-foreground">{m.system_mcp_section_hint()}</p>
    <div class="mt-3 grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(15rem,1fr))]" data-testid="plugins-grid-third-party">
      {#each remotes as plugin (plugin.manifest.id)}
        {@render card(plugin)}
      {/each}
    </div>
  {/if}
</div>

<AddMcpServerDialog bind:open={adding} onAdded={() => (pollUntil = Date.now() + 60_000)} />
