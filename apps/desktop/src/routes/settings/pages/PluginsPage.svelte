<script lang="ts">
  // 左侧栏的「插件」（ADR 038，2026-11-24 玩家定的）：像应用启动器，每个插件一张卡片，对应 MCP Apps 里的一个 App。
  // 卡片上有图标、名字、一句话、开关（关着的卡片变淡）；点卡片（不是开关）进这个插件的 App，App 占满内容区、
  // 用它自己的界面。以后的第三方插件（别的仓库的 MCP App，沙盒 iframe 显示 `ui://` 资源）也是这里的一张卡片，
  // 打开后放进同一个 App 位置（PluginPage.svelte）。插件自己的数据开关和风险提示在它自己的 App 里。
  import { onMount } from "svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { listPlugins, pluginDescription, pluginEnabled, pluginName, pluginsStore, setPluginEnabled } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });

  async function toggle(id: string, value: boolean) {
    try { await setPluginEnabled(id, value); } catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
  }
</script>

<div class="mx-auto w-full max-w-5xl px-8 pt-7 pb-4" data-testid="plugins-page">
  <h1 class="m-0 text-xl font-semibold tracking-tight text-foreground">{m.shell_sidebar_plugins()}</h1>
  <p class="mt-1.5 mb-0 max-w-3xl text-xs leading-relaxed text-muted-foreground">{m.system_plugins_description({ companion: $companionName })}</p>

  <div class="mt-6 grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(15rem,1fr))]" data-testid="plugins-grid">
    {#each $pluginsStore as plugin (plugin.manifest.id)}
      {@const manifest = plugin.manifest}
      {@const ui = PLUGIN_UI[manifest.id]}
      {@const Icon = ui?.icon}
      {@const on = pluginEnabled($settingsStore, manifest)}
      {@const name = pluginName(manifest)}
      <div class="group relative flex min-h-36 flex-col rounded-2xl border border-hairline bg-card p-4 transition-[border-color,box-shadow,opacity]
          hover:border-primary/45 hover:shadow-sm {on ? '' : 'opacity-60 hover:opacity-90'}"
        data-testid={`plugins-card-${manifest.id}`} data-enabled={on}>
        <!-- 整张卡片可点（开关除外）：打开这个插件的 App -->
        <button type="button" class="absolute inset-0 rounded-2xl focus-visible:outline-2 focus-visible:outline-primary disabled:cursor-default"
          aria-label={m.shell_plugins_open_aria({ name })} data-testid={`plugins-card-${manifest.id}-open`} disabled={!ui}
          onclick={() => openTarget({ page: "plugin", id: manifest.id })}></button>
        <div class="pointer-events-none relative flex items-start justify-between gap-3">
          <div class="grid size-10 shrink-0 place-items-center rounded-xl {on ? 'bg-primary/12 text-primary' : 'bg-muted text-muted-foreground'}">
            {#if Icon}<Icon class="size-5" />{/if}
          </div>
          <div class="pointer-events-auto relative z-10">
            <!-- 受控：显示的总是设置里的值，没存上时不会停在错误的位置 -->
            <Switch bind:checked={() => on, (next) => void toggle(manifest.id, next)}
              aria-label={m.system_plugins_enable({ name })} data-testid={`plugins-card-${manifest.id}-enabled`} />
          </div>
        </div>
        <p class="pointer-events-none relative mt-3 mb-0 flex items-center gap-2 text-sm font-semibold text-foreground">
          {name}
          {#if !on}<span class="rounded bg-muted px-1.5 text-[10px] font-normal text-muted-foreground" data-testid={`plugins-card-${manifest.id}-off`}>{m.system_plugins_off_badge()}</span>{/if}
        </p>
        <p class="pointer-events-none relative mt-1 mb-0 line-clamp-3 text-xs leading-relaxed text-muted-foreground">{pluginDescription(manifest)}</p>
      </div>
    {/each}
  </div>
</div>
