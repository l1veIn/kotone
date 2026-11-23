<script lang="ts">
  // 「设置 → 权限」里的内置浏览器（ADR 031）：清除你在聊天页右侧网页里登录过的账号（网易云等）；
  // 浏览器插件（已解压的插件文件夹，Chrome / Edge 插件格式；WebView2 的 AddBrowserExtension）。
  import { onMount } from "svelte";
  import { FolderOpen, Puzzle, Trash2 } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import * as Dialog from "$lib/components/ui/dialog";
  import {
    browserClearData, browserExtensionAdd, browserExtensionRemove, browserExtensions, browserExtensionToggle,
    type BrowserExtension,
  } from "../../../../lib/browser";
  import { isTauri } from "../../../../lib/ipc";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let confirming = $state(false);
  let clearing = $state(false);

  async function clear() {
    clearing = true;
    try {
      await browserClearData();
      confirming = false;
      toast(true, m.companion_browser_cleared());
    } catch (error) {
      toast(false, m.companion_browser_clear_failed({ error: errText(error) }));
    } finally {
      clearing = false;
    }
  }

  // ---- 浏览器插件 ----
  let extensions = $state<BrowserExtension[]>([]);
  let loadError = $state("");
  let busy = $state<string | null>(null);
  /** 界面预览（没有 Tauri）没有选文件夹的对话框：直接填路径。 */
  let manualPath = $state("");

  async function reload() {
    try {
      extensions = await browserExtensions();
      loadError = "";
    } catch (error) {
      loadError = errText(error);
    }
  }
  onMount(() => { void reload(); });

  async function add(path: string) {
    const value = path.trim();
    if (!value || busy) return;
    busy = "add";
    try {
      await browserExtensionAdd(value);
      manualPath = "";
      toast(true, m.companion_browser_extension_added());
      await reload();
    } catch (error) {
      toast(false, m.companion_browser_extension_add_failed({ error: errText(error) }));
    } finally {
      busy = null;
    }
  }
  async function pick() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({ directory: true, title: m.companion_browser_extension_pick_title() }).catch(() => null);
    if (typeof dir === "string" && dir) await add(dir);
  }
  async function toggle(ext: BrowserExtension, enabled: boolean) {
    busy = ext.id;
    try {
      await browserExtensionToggle(ext.id, enabled);
      await reload();
    } catch (error) {
      toast(false, enabled
        ? m.companion_browser_extension_enable_failed({ error: errText(error) })
        : m.companion_browser_extension_disable_failed({ error: errText(error) }));
    } finally {
      busy = null;
    }
  }
  async function remove(ext: BrowserExtension) {
    busy = ext.id;
    try {
      await browserExtensionRemove(ext.id);
      await reload();
    } catch (error) {
      toast(false, m.companion_browser_extension_remove_failed({ error: errText(error) }));
    } finally {
      busy = null;
    }
  }
</script>

<SettingsSection title={m.companion_browser_title()} id="settings-permissions-browser"
  description={m.companion_browser_desc()} testid="browser-data-settings">
  <SettingsRow label={m.companion_browser_clear_label()} description={m.companion_browser_clear_desc()}>
    {#snippet control()}
      <Button size="sm" variant="outline" class="text-destructive hover:bg-destructive/10 hover:text-destructive"
        data-testid="browser-clear-data" onclick={() => (confirming = true)}>{m.companion_browser_clear_button()}</Button>
    {/snippet}
  </SettingsRow>
</SettingsSection>

<SettingsSection title={m.companion_browser_extensions_title()} id="settings-permissions-browser-extensions"
  description={m.companion_browser_extensions_desc()}
  testid="browser-extensions-settings">
  {#snippet actions()}
    {#if isTauri}
      <Button size="sm" variant="outline" class="gap-1.5" data-testid="browser-extension-add" disabled={busy === "add"} onclick={() => void pick()}>
        <FolderOpen class="size-3.5" />{m.companion_browser_extension_add_folder()}
      </Button>
    {/if}
  {/snippet}
  {#if !isTauri}
    <!-- 界面预览没有选文件夹的对话框：填路径 -->
    <form class="flex gap-2 py-2" onsubmit={(e) => { e.preventDefault(); void add(manualPath); }}>
      <Input bind:value={manualPath} placeholder={m.companion_browser_extension_path_placeholder()} class="h-8 text-xs" data-testid="browser-extension-path" />
      <Button type="submit" size="sm" variant="outline" data-testid="browser-extension-add" disabled={!manualPath.trim() || busy === "add"}>{m.companion_browser_extension_add()}</Button>
    </form>
  {/if}
  {#if loadError}<p role="alert" class="m-0 py-2 text-xs text-destructive">{loadError}</p>{/if}
  {#if extensions.length}
    <ul class="m-0 list-none divide-y divide-hairline p-0" data-testid="browser-extensions">
      {#each extensions as ext (ext.id)}
        <li class="flex min-w-0 items-center gap-3 py-2.5" data-testid="browser-extension" data-extension-id={ext.id}>
          <Puzzle class="size-4 shrink-0 text-muted-foreground" />
          <div class="min-w-0 flex-1">
            <p class="m-0 truncate text-[13px] font-medium text-foreground" data-testid="browser-extension-name">{ext.name}</p>
            <p class="m-0 truncate text-xs text-muted-foreground" title={ext.path}>{ext.path}</p>
          </div>
          <Switch aria-label={ext.enabled ? m.companion_browser_extension_disable_aria({ name: ext.name }) : m.companion_browser_extension_enable_aria({ name: ext.name })} data-testid="browser-extension-toggle"
            disabled={busy === ext.id} checked={ext.enabled} onCheckedChange={(value) => void toggle(ext, value)} />
          <Button size="icon" variant="ghost" class="size-7 shrink-0 text-muted-foreground hover:text-destructive"
            aria-label={m.companion_browser_extension_remove_aria({ name: ext.name })} title={m.companion_browser_extension_remove()} data-testid="browser-extension-remove"
            disabled={busy === ext.id} onclick={() => void remove(ext)}><Trash2 class="size-3.5" /></Button>
        </li>
      {/each}
    </ul>
  {:else if !loadError}
    <p class="m-0 py-2 text-xs text-muted-foreground" data-testid="browser-extensions-empty">{m.companion_browser_extensions_empty()}</p>
  {/if}
</SettingsSection>

<Dialog.Root open={confirming} onOpenChange={(open) => { if (!open && !clearing) confirming = false; }}>
  <Dialog.Content class="ui-scope" data-testid="browser-clear-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.companion_browser_clear_dialog_title()}</Dialog.Title>
      <Dialog.Description>{m.companion_browser_clear_dialog_desc()}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={clearing} onclick={() => (confirming = false)}>{m.companion_cancel()}</Button>
      <Button variant="destructive" size="sm" data-testid="browser-clear-confirm" disabled={clearing} onclick={() => void clear()}>{m.companion_browser_clear_confirm()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
