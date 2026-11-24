<script lang="ts">
  // 第三方插件的设置（插件页顶栏的“设置”，和内置插件同一个位置）：连接（运行什么 / 连哪里）、Key（`${input:名字}`
  // 的值，存进 Windows 的安全存储，不进 mcp.json）、信任（它说只读的工具要不要先问）、风险、删掉。
  import { ShieldAlert, Trash2 } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import McpTrustFacts from "./McpTrustFacts.svelte";
  import { reconnectServer, remoteOf, removeServer, setServerKey, trustServer } from "$lib/mcpServers";
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

  <section class="space-y-2">
    <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_settings_risk()}</h2>
    <div class="flex gap-2 rounded-xl border border-warning/45 bg-warning/10 p-3 text-xs leading-relaxed text-foreground" data-testid="remote-plugin-risk">
      <ShieldAlert class="mt-0.5 size-4 shrink-0 text-warning" /><span>{m.system_mcp_risk()}</span>
    </div>
  </section>

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
