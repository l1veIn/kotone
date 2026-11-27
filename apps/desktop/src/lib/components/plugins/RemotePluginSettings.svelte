<script lang="ts">
  // 第三方插件的设置（插件页顶栏的“设置”，和内置插件同一个位置）：连接（运行什么 / 连哪里）、Key（`${input:名字}`
  // 的值，存进 Windows 的安全存储，不进 mcp.json）、信任（它说只读的工具要不要先问）、Kotone 清单（插件标准 0.1：从哪来，
  // 给作者看的小字：不认识的字段、清单的问题、标准比这一版新）、删掉。
  // 风险不在这里摆（ADR 043：一个插件一个弹窗）：顶栏的“风险和许可”里是“它是别人写的程序”和它清单里的 `risk`。
  import { Trash2 } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import McpTrustFacts from "./McpTrustFacts.svelte";
  import { reconnectServer, remoteOf, removeServer, setServerKey, trustServer, type ManifestNote } from "$lib/mcpServers";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { openTarget } from "$lib/stores/navigation";
  import { errText, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin }: PluginPageProps = $props();
  const remote = $derived(remoteOf(plugin));
  const id = $derived(plugin.manifest.id);
  let values = $state<Record<string, string>>({});
  let busy = $state(false);
  let confirmingRemove = $state(false);
  const kotone = $derived(remote?.kotoneManifest ?? null);
  /** 有清单，或者有要给作者看的（给了但读不出来、本地文件坏了）：才有“Kotone 清单”这一节。 */
  const showManifest = $derived(!!kotone && (kotone.source !== "none" || kotone.unknown.length > 0 || kotone.problems.length > 0));

  const sourceText = (source: string, standard: string) =>
    ({
      server: m.system_mcp_manifest_from_server({ standard }),
      preset: m.system_mcp_manifest_from_preset(),
      local: m.system_mcp_manifest_from_local(),
    })[source] ?? m.system_mcp_manifest_none();
  /** 提示出在哪一层（服务器给的 / Kotone 带的 / 你的 manifest.json）。 */
  const where = (note: ManifestNote) =>
    ({
      server: m.system_mcp_manifest_note_server(),
      preset: m.system_mcp_manifest_note_preset(),
      local: m.system_mcp_manifest_note_local(),
      none: "",
    })[note.from];

  async function run(action: () => Promise<void>) {
    busy = true;
    try { await action(); } catch (error) { toast(false, errText(error)); } finally { busy = false; }
  }

  const saveKey = (name: string, value: string | null) => run(async () => {
    await setServerKey(id, name, value);
    values[name] = "";
    if (value) toast(true, m.system_mcp_key_saved());
  });

  const reconnect = () => run(async () => {
    const error = await reconnectServer(id);
    toast(!error, error ? m.system_mcp_reconnect_failed({ error }) : m.system_mcp_reconnected());
  });

  const remove = () => run(async () => {
    await removeServer(id);
    confirmingRemove = false;
    toast(true, m.system_mcp_removed({ name: id }));
    openTarget({ page: "plugins" });
  });
</script>

<div class="mx-auto w-full max-w-3xl space-y-6 px-6 py-6" data-testid="remote-plugin-settings" data-plugin-id={id}>
  <section class="space-y-2">
    <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_settings_connection()}</h2>
    {#if remote?.config}
      <McpTrustFacts facts={remote.config} testid="remote-plugin-facts" />
    {:else if remote?.configError}
      <p class="m-0 text-xs text-destructive" data-testid="remote-plugin-config-error">{m.system_mcp_config_error({ error: remote.configError })}</p>
    {/if}
    <div class="flex items-center gap-2 text-xs">
      {#if remote?.status === "failed" && remote.error}<span class="text-destructive">{remote.error}</span>{/if}
      <Button size="sm" variant="outline" class="ml-auto h-7 text-xs" disabled={busy} data-testid="remote-plugin-settings-reconnect" onclick={() => void reconnect()}>{m.system_mcp_reconnect()}</Button>
    </div>
  </section>

  {#if remote?.keys.length}
    <section class="space-y-2" data-testid="remote-plugin-keys">
      <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_settings_keys()}</h2>
      {#each remote.keys as key (key.name)}
        <div class="flex flex-wrap items-center gap-2" data-testid="remote-plugin-key" data-key={key.name} data-filled={key.filled}>
          <code class="w-48 truncate font-mono text-xs text-foreground">{key.name}</code>
          <span class="text-xs {key.filled ? 'text-muted-foreground' : 'text-warning'}">{key.filled ? m.system_mcp_key_filled() : m.system_mcp_key_missing()}</span>
          <Input type="password" autocomplete="off" class="h-8 w-56 text-xs" placeholder={m.system_mcp_key_placeholder({ name: key.name })}
            bind:value={values[key.name]} data-testid="remote-plugin-key-input" />
          <Button size="sm" class="h-8 text-xs" disabled={busy || !values[key.name]?.trim()} data-testid="remote-plugin-key-save"
            onclick={() => void saveKey(key.name, values[key.name])}>{m.system_mcp_key_save()}</Button>
          {#if key.filled}
            <Button size="sm" variant="ghost" class="h-8 text-xs" disabled={busy} data-testid="remote-plugin-key-clear"
              onclick={() => void saveKey(key.name, null)}>{m.system_mcp_key_clear()}</Button>
          {/if}
        </div>
      {/each}
    </section>
  {/if}

  <section class="space-y-2" data-testid="remote-plugin-trust" data-trusted={remote?.trusted}>
    <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_settings_trust()}</h2>
    <p class="m-0 text-xs leading-relaxed text-muted-foreground">{remote?.trusted ? m.system_mcp_trusted_yes() : m.system_mcp_trusted_no()}</p>
    {#if remote?.trusted}
      <Button size="sm" variant="outline" class="h-7 text-xs" disabled={busy} data-testid="remote-plugin-untrust" onclick={() => void run(() => trustServer(id, false))}>{m.system_mcp_untrust_button()}</Button>
    {:else}
      <Button size="sm" class="h-7 text-xs" disabled={busy || !remote?.config} data-testid="remote-plugin-trust-button" onclick={() => void run(() => trustServer(id, true))}>{m.system_mcp_trust_button()}</Button>
    {/if}
  </section>

  {#if kotone && showManifest}
    <section class="space-y-1.5" data-testid="remote-plugin-manifest" data-source={kotone.source}>
      <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_manifest_title()}</h2>
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="remote-plugin-manifest-source">
        {sourceText(kotone.source, kotone.manifest.standard)}{#if kotone.local && kotone.source !== "local"}{m.system_mcp_manifest_local_added()}{/if}
      </p>
      <!-- 下面是给插件作者看的小字 -->
      {#if kotone.newerStandard}
        <p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="remote-plugin-manifest-newer">
          {m.system_mcp_manifest_newer({ standard: kotone.manifest.standard, host: kotone.hostStandard })}</p>
      {/if}
      {#if kotone.unknownGames.length}
        <p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="remote-plugin-manifest-unknown-games">
          {m.system_mcp_manifest_unknown_games({ games: kotone.unknownGames.join(m.system_mcp_manifest_separator()) })}</p>
      {/if}
      {#if kotone.unknown.length}
        <p class="m-0 break-all text-[11px] leading-relaxed text-muted-foreground" data-testid="remote-plugin-manifest-unknown">
          {m.system_mcp_manifest_unknown({ fields: kotone.unknown.map((n) => (n.from === "local" ? m.system_mcp_manifest_field_in({ field: n.text, where: where(n) }) : n.text)).join(m.system_mcp_manifest_separator()) })}</p>
      {/if}
      {#if kotone.problems.length}
        <div class="space-y-0.5" data-testid="remote-plugin-manifest-problems">
          <p class="m-0 text-[11px] text-muted-foreground">{m.system_mcp_manifest_problems()}</p>
          <ul class="m-0 list-disc space-y-0.5 pl-4 text-[11px] leading-relaxed text-muted-foreground">
            {#each kotone.problems as note, i (i)}
              <li class="break-all" data-testid="remote-plugin-manifest-problem" data-from={note.from}>{m.system_mcp_manifest_problem({ where: where(note), text: note.text })}</li>
            {/each}
          </ul>
        </div>
      {/if}
      {#if kotone.source === "local" || kotone.local || kotone.problems.some((n) => n.from === "local")}
        <p class="m-0 break-all text-[11px] text-muted-foreground" data-testid="remote-plugin-manifest-path">{m.system_mcp_manifest_path({ path: kotone.localPath })}</p>
      {/if}
    </section>
  {/if}

  <section>
    <Button size="sm" variant="ghost" class="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={busy}
      data-testid="remote-plugin-remove" onclick={() => (confirmingRemove = true)}><Trash2 class="size-3.5" />{m.system_mcp_remove()}</Button>
  </section>
</div>

<Dialog.Root open={confirmingRemove} onOpenChange={(open) => { if (!open && !busy) confirmingRemove = false; }}>
  <Dialog.Content class="ui-scope" data-testid="remote-plugin-remove-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.system_mcp_remove()}</Dialog.Title>
      <Dialog.Description>{m.system_mcp_remove_confirm({ name: id })}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="ghost" disabled={busy} onclick={() => (confirmingRemove = false)}>{m.system_mcp_cancel()}</Button>
      <Button variant="destructive" disabled={busy} data-testid="remote-plugin-remove-confirm" onclick={() => void remove()}>{m.system_mcp_remove()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
