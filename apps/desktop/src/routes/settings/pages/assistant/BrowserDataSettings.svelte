<script lang="ts">
  // 「设置 → 权限」里的内置浏览器（ADR 031）：放歌用哪个网站（agent.musicSite）；
  // 清除你在聊天页右侧网页里登录过的账号（网易云等）；
  // 浏览器插件（已解压的插件文件夹，Chrome / Edge 插件格式；WebView2 的 AddBrowserExtension）。
  import { onMount } from "svelte";
  import { FolderOpen, Puzzle, Trash2 } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import * as Dialog from "$lib/components/ui/dialog";
  import {
    browserClearData, browserExtensionAdd, browserExtensionRemove, browserExtensions, browserExtensionToggle,
    type BrowserExtension,
  } from "../../../../lib/browser";
  import { isTauri } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";

  /** 放歌用的网站（和 kotone-core 的 MUSIC_SITES 一致）：她只在这个网站上找歌，搜不到就告诉你。 */
  const MUSIC_SITES = [
    { value: "netease", label: "网易云音乐" },
    { value: "qq", label: "QQ 音乐" },
    { value: "kugou", label: "酷狗音乐" },
    { value: "kuwo", label: "酷我音乐" },
  ];
  const musicSite = $derived($settingsStore?.agent?.musicSite || "netease");
  function setMusicSite(value: string) {
    const name = MUSIC_SITES.find((site) => site.value === value)?.label ?? value;
    void patchSettings({ agent: { musicSite: value } }, `放歌改用${name}`);
  }

  let confirming = $state(false);
  let clearing = $state(false);

  async function clear() {
    clearing = true;
    try {
      await browserClearData();
      confirming = false;
      toast(true, "已清除内置浏览器的登录和网页数据");
    } catch (error) {
      toast(false, `清除失败：${errText(error)}`);
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
      toast(true, "已添加插件，重新打开网页后生效");
      await reload();
    } catch (error) {
      toast(false, `没能添加插件：${errText(error)}`);
    } finally {
      busy = null;
    }
  }
  async function pick() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({ directory: true, title: "选择已解压的浏览器插件文件夹（里面有 manifest.json）" }).catch(() => null);
    if (typeof dir === "string" && dir) await add(dir);
  }
  async function toggle(ext: BrowserExtension, enabled: boolean) {
    busy = ext.id;
    try {
      await browserExtensionToggle(ext.id, enabled);
      await reload();
    } catch (error) {
      toast(false, `没能${enabled ? "启用" : "停用"}插件：${errText(error)}`);
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
      toast(false, `没能移除插件：${errText(error)}`);
    } finally {
      busy = null;
    }
  }
</script>

<SettingsSection title="放歌" id="settings-permissions-music"
  description="在聊天页右侧的网页里放歌。她只在你选的网站上找；搜不到会告诉你，不会自己换网站。第一次要在网页上扫码登录。"
  testid="music-site-settings">
  <SettingsRow label="用哪个网站">
    {#snippet control()}
      <SelectField class="w-36" size="sm" ariaLabel="放歌用的网站" testid="music-site" value={musicSite} options={MUSIC_SITES}
        onchange={setMusicSite} />
    {/snippet}
  </SettingsRow>
</SettingsSection>

<SettingsSection title="内置浏览器" id="settings-permissions-browser"
  description="聊天页右侧的网页（放歌、查东西）。你在里面登录过的账号只存在这台电脑的 Kotone 里。" testid="browser-data-settings">
  <SettingsRow label="清除浏览器登录" description="退出在右侧网页里登录过的所有账号，删掉它的网页数据。正在放的歌会停。">
    {#snippet control()}
      <Button size="sm" variant="outline" class="text-destructive hover:bg-destructive/10 hover:text-destructive"
        data-testid="browser-clear-data" onclick={() => (confirming = true)}>清除…</Button>
    {/snippet}
  </SettingsRow>
</SettingsSection>

<SettingsSection title="浏览器插件" id="settings-permissions-browser-extensions"
  description="给右侧网页装插件（例如去广告）。只支持已解压的插件文件夹（Chrome / Edge 插件格式），添加、停用后重新打开网页时生效。"
  testid="browser-extensions-settings">
  {#snippet actions()}
    {#if isTauri}
      <Button size="sm" variant="outline" class="gap-1.5" data-testid="browser-extension-add" disabled={busy === "add"} onclick={() => void pick()}>
        <FolderOpen class="size-3.5" />添加插件文件夹…
      </Button>
    {/if}
  {/snippet}
  {#if !isTauri}
    <!-- 界面预览没有选文件夹的对话框：填路径 -->
    <form class="flex gap-2 py-2" onsubmit={(e) => { e.preventDefault(); void add(manualPath); }}>
      <Input bind:value={manualPath} placeholder="插件文件夹路径，例如 D:\extensions\ublock" class="h-8 text-xs" data-testid="browser-extension-path" />
      <Button type="submit" size="sm" variant="outline" data-testid="browser-extension-add" disabled={!manualPath.trim() || busy === "add"}>添加</Button>
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
          <Switch aria-label={`${ext.enabled ? "停用" : "启用"}插件 ${ext.name}`} data-testid="browser-extension-toggle"
            disabled={busy === ext.id} checked={ext.enabled} onCheckedChange={(value) => void toggle(ext, value)} />
          <Button size="icon" variant="ghost" class="size-7 shrink-0 text-muted-foreground hover:text-destructive"
            aria-label={`移除插件 ${ext.name}`} title="移除" data-testid="browser-extension-remove"
            disabled={busy === ext.id} onclick={() => void remove(ext)}><Trash2 class="size-3.5" /></Button>
        </li>
      {/each}
    </ul>
  {:else if !loadError}
    <p class="m-0 py-2 text-xs text-muted-foreground" data-testid="browser-extensions-empty">还没有装插件。</p>
  {/if}
</SettingsSection>

<Dialog.Root open={confirming} onOpenChange={(open) => { if (!open && !clearing) confirming = false; }}>
  <Dialog.Content class="ui-scope" data-testid="browser-clear-dialog">
    <Dialog.Header>
      <Dialog.Title>清除内置浏览器的登录？</Dialog.Title>
      <Dialog.Description>在聊天页右侧网页里登录过的账号（例如网易云音乐）都会退出，下次要用时再登录一次。</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={clearing} onclick={() => (confirming = false)}>取消</Button>
      <Button variant="destructive" size="sm" data-testid="browser-clear-confirm" disabled={clearing} onclick={() => void clear()}>清除</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
