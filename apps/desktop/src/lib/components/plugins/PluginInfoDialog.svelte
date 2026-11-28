<script lang="ts">
  // 「插件信息」（ADR 049 D7 第 3 条，第 0 期）：一个只读的弹窗，入口在插件卡片的“⋯”菜单和插件页顶栏的名字旁边。
  // **只显示现在有的，没有的那一行不出现**：
  // - 头像、名字、版本、一句话说明；
  // - 内置插件：作者“Kotone”、代码仓库（l1veIn/kotone 里的 plugins/<id>）、许可证 MIT、版本 = Kotone 的版本（宿主统一填，
  //   `PluginInfo.about`）；照着写的开源项目（清单的 `[[references]]`）列在“参考”，旁边链接 THIRD_PARTY_NOTICES.md；
  // - 第三方插件：服务器握手时说的名字、版本、说明、主页（`serverInfo.websiteUrl`），来源是你粘贴或导入的配置；
  //   作者、许可证这一期还没有（等插件包，ADR 049 第 2 期），不显示；
  // - 它能做什么：属于哪些游戏，几个工具、页面、事件、推送（点开看名字）；
  // - 它会运行什么（第三方）：程序的位置或网址（信任提示里那一块 `McpTrustFacts`）；“风险和许可”打开现有的弹窗；
  // - 给作者看的小字：插件 id；第三方另有它清单写的插件标准版本。
  // 链接只认 `https://`，用系统浏览器打开（`openExternal`）。
  import { tick } from "svelte";
  import { ShieldAlert } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { listProfiles, openExternal } from "$lib/ipc";
  import { hasRiskNotes, pluginDescription, pluginName, pluginText, pushTitle, thirdPartyRisk, type PluginInfo } from "$lib/plugins";
  import { remoteIcon, remoteOf, remoteTitle } from "$lib/mcpServers";
  import PluginLogo from "./PluginLogo.svelte";
  import McpTrustFacts from "./McpTrustFacts.svelte";
  import PluginRiskDialog from "./PluginRiskDialog.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin, open = $bindable(false) }: { plugin: PluginInfo; open?: boolean } = $props();

  const manifest = $derived(plugin.manifest);
  const id = $derived(manifest.id);
  const name = $derived(pluginName(manifest));
  const remote = $derived(remoteOf(plugin));
  const about = $derived(remote ? null : (plugin.about ?? null));
  const server = $derived(remote?.server ?? null);

  /** 只认 `https://` 的完整地址。 */
  const https = (url: string | null | undefined): string | null =>
    typeof url === "string" && /^https:\/\/\S+$/.test(url.trim()) ? url.trim() : null;
  /** 链接显示成什么：去掉 https:// 和末尾的斜杠。 */
  const shown = (url: string) => url.replace(/^https:\/\//, "").replace(/\/$/, "");

  const version = $derived((remote ? server?.version : about?.version)?.trim() || null);
  const subtitle = $derived(remote ? remoteTitle(plugin) : null);
  const description = $derived(remote ? server?.description?.trim() || pluginDescription(manifest) : pluginDescription(manifest));
  const homepage = $derived(https(server?.websiteUrl));
  const repository = $derived(https(about?.repository));
  const references = $derived(remote ? [] : (manifest.references ?? []).filter((r) => r.name.trim() && https(r.url)));
  const notices = $derived(https(about?.notices));

  /** 它能做什么：每样的名字（没有的那样不显示）。 */
  type Group = { key: string; label: (count: number) => string; names: string[] };
  const groups = $derived.by((): Group[] => {
    const tools = remote ? (remote.toolList ?? []).map((t) => t.title?.trim() || t.name) : plugin.tools.map((t) => t.name);
    const pages = remote
      ? remote.pages.map((p) => p.title?.trim() || p.uri)
      : manifest.pages.map((p) => p.title?.trim() || p.uri);
    const events = remote
      ? (remote.kotoneManifest?.manifest.events ?? []).map((e) => e.label?.trim() || e.name)
      : (manifest.events ?? []).map((e) => pluginText(id, `event_${e.type.slice(e.type.indexOf(".") + 1)}`, e.label));
    const pushes = remote ? [] : (manifest.pushes ?? []).map((p) => pushTitle(id, p));
    const all: Group[] = [
      { key: "tools", label: (count) => m.system_plugins_info_tools({ count }), names: tools },
      { key: "pages", label: (count) => m.system_plugins_info_pages({ count }), names: pages },
      { key: "events", label: (count) => m.system_plugins_info_events({ count }), names: events },
      { key: "pushes", label: (count) => m.system_plugins_info_pushes({ count }), names: pushes },
    ];
    return all.filter((g) => g.names.length > 0);
  });

  /** 游戏配置 id → 名字（打开时读一次；读不到就显示 id）。 */
  let gameNames = $state<Record<string, string>>({});
  $effect(() => {
    if (!open || !manifest.games.length) return;
    void listProfiles()
      .then((profiles) => { gameNames = Object.fromEntries((profiles ?? []).map((p) => [p.id, p.displayName])); })
      .catch(() => {});
  });
  const games = $derived(manifest.games.map((g) => gameNames[g] || g));

  /** 有“风险和许可”要看（数据开关的风险提示、许可和出处；第三方插件总有）：和插件页顶栏一样。 */
  const hasRisk = $derived(hasRiskNotes(manifest) || thirdPartyRisk(plugin).thirdParty);
  let riskOpen = $state(false);
  async function openRisk() {
    open = false;
    await tick();
    riskOpen = true;
  }
  const standard = $derived(remote?.kotoneManifest && remote.kotoneManifest.source !== "none" ? remote.kotoneManifest.manifest.standard : null);
</script>

{#snippet link(url: string, testid: string)}
  <button type="button" class="min-w-0 break-all text-left text-primary underline-offset-2 hover:underline"
    data-testid={testid} data-url={url} onclick={() => void openExternal(url).catch(() => {})}>{shown(url)}</button>
{/snippet}

<Dialog.Root bind:open>
  <Dialog.Content class="ui-scope max-h-[85vh] overflow-y-auto sm:max-w-lg" data-testid={`plugin-info-dialog-${id}`} data-remote={!!remote}>
    <Dialog.Header>
      <div class="flex items-start gap-3 text-left">
        <PluginLogo {id} {name} src={remote ? remoteIcon(plugin) : null} size={44} testid="plugin-info-logo" />
        <div class="min-w-0 flex-1">
          <Dialog.Title class="flex flex-wrap items-center gap-2">
            <span data-testid="plugin-info-name">{name}</span>
            {#if version}<span class="rounded bg-muted px-1.5 text-[11px] font-normal text-muted-foreground" data-testid="plugin-info-version">{m.system_plugins_info_version({ version })}</span>{/if}
          </Dialog.Title>
          {#if subtitle}<p class="m-0 mt-0.5 truncate text-xs text-muted-foreground" data-testid="plugin-info-subtitle">{subtitle}</p>{/if}
          <Dialog.Description class="mt-1 text-xs leading-relaxed" data-testid="plugin-info-description">{description}</Dialog.Description>
        </div>
      </div>
    </Dialog.Header>

    <!-- 作者、仓库、主页、许可证、来源、参考：有的才有那一行 -->
    <dl class="m-0 grid grid-cols-[5.5rem_1fr] gap-x-3 gap-y-1.5 text-xs" data-testid="plugin-info-facts">
      {#if about?.author}
        <dt class="text-muted-foreground">{m.system_plugins_info_author()}</dt>
        <dd class="m-0 text-foreground" data-testid="plugin-info-author">{about.author}</dd>
      {/if}
      {#if repository}
        <dt class="text-muted-foreground">{m.system_plugins_info_repository()}</dt>
        <dd class="m-0 min-w-0">{@render link(repository, "plugin-info-repository")}</dd>
      {/if}
      {#if homepage}
        <dt class="text-muted-foreground">{m.system_plugins_info_homepage()}</dt>
        <dd class="m-0 min-w-0">{@render link(homepage, "plugin-info-homepage")}</dd>
      {/if}
      {#if about?.license}
        <dt class="text-muted-foreground">{m.system_plugins_info_license()}</dt>
        <dd class="m-0 text-foreground" data-testid="plugin-info-license">{about.license}</dd>
      {/if}
      <dt class="text-muted-foreground">{m.system_plugins_info_source()}</dt>
      <dd class="m-0 text-foreground" data-testid="plugin-info-source" data-source={remote ? "config" : "builtin"}>
        {remote ? m.system_plugins_info_source_config() : m.system_plugins_info_source_builtin()}</dd>
      {#if references.length}
        <dt class="text-muted-foreground">{m.system_plugins_info_references()}</dt>
        <dd class="m-0 flex min-w-0 flex-wrap gap-x-3 gap-y-0.5" data-testid="plugin-info-references">
          {#each references as reference (reference.url)}
            <button type="button" class="text-primary underline-offset-2 hover:underline" data-testid="plugin-info-reference" data-url={reference.url}
              onclick={() => void openExternal(reference.url).catch(() => {})}>{reference.name}</button>
          {/each}
          {#if notices}
            <button type="button" class="text-muted-foreground underline underline-offset-2 hover:text-foreground" data-testid="plugin-info-notices"
              data-url={notices} onclick={() => void openExternal(notices).catch(() => {})}>{m.system_plugins_info_notices()}</button>
          {/if}
        </dd>
      {/if}
    </dl>

    {#if games.length || groups.length}
      <section class="space-y-1.5 border-t border-hairline pt-3" data-testid="plugin-info-abilities">
        <h3 class="m-0 text-xs font-semibold text-foreground">{m.system_plugins_info_abilities()}</h3>
        {#if games.length}
          <p class="m-0 text-xs text-muted-foreground" data-testid="plugin-info-games">{m.system_plugins_info_games({ games: games.join(m.system_mcp_manifest_separator()) })}</p>
        {/if}
        {#each groups as group (group.key)}
          <!-- 简单折叠：数字，点开看名字 -->
          <details class="text-xs" data-testid={`plugin-info-${group.key}`} data-count={group.names.length}>
            <summary class="cursor-pointer text-foreground select-none">{group.label(group.names.length)}</summary>
            <ul class="m-0 mt-1 list-none space-y-0.5 pl-4 text-[11px] text-muted-foreground">
              {#each group.names as item, i (i)}<li class="break-all" data-testid={`plugin-info-${group.key}-item`}>{item}</li>{/each}
            </ul>
          </details>
        {/each}
      </section>
    {/if}

    {#if remote?.config}
      <section class="space-y-1.5 border-t border-hairline pt-3" data-testid="plugin-info-runs">
        <h3 class="m-0 text-xs font-semibold text-foreground">{m.system_plugins_info_runs()}</h3>
        <McpTrustFacts facts={remote.config} testid="plugin-info-facts-runs" />
      </section>
    {/if}

    {#if hasRisk}
      <div class={remote?.config ? "-mt-2" : "border-t border-hairline pt-3"}>
        <button type="button" data-testid="plugin-info-risk-link"
          class="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          onclick={() => void openRisk()}><ShieldAlert class="size-3.5" />{m.system_plugins_risk_link()}</button>
      </div>
    {/if}

    <!-- 给插件作者看的小字 -->
    <p class="m-0 flex flex-wrap gap-x-3 font-mono text-[11px] text-muted-foreground" data-testid="plugin-info-small">
      <span data-testid="plugin-info-id">{m.system_plugins_info_id({ id })}</span>
      {#if standard}<span data-testid="plugin-info-standard">{m.system_plugins_info_standard({ standard })}</span>{/if}
    </p>

    <Dialog.Footer>
      <Button size="sm" variant="outline" data-testid="plugin-info-close" onclick={() => (open = false)}>{m.system_plugins_info_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>

{#if hasRisk}<PluginRiskDialog {plugin} bind:open={riskOpen} />{/if}
