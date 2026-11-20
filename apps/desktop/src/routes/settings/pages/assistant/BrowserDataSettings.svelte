<script lang="ts">
  // 「设置 → 权限」里的内置浏览器（ADR 031）：清除你在聊天页右侧网页里登录过的账号（网易云等）。
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { browserClearData } from "../../../../lib/browser";
  import { errText, toast } from "../../../../lib/stores/ui";

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
</script>

<SettingsSection title="内置浏览器" id="settings-permissions-browser"
  description="聊天页右侧的网页（放歌、查东西）。你在里面登录过的账号只存在这台电脑的 Kotone 里。" testid="browser-data-settings">
  <SettingsRow label="清除浏览器登录" description="退出在右侧网页里登录过的所有账号，删掉它的网页数据。正在放的歌会停。">
    {#snippet control()}
      <Button size="sm" variant="outline" class="text-destructive hover:bg-destructive/10 hover:text-destructive"
        data-testid="browser-clear-data" onclick={() => (confirming = true)}>清除…</Button>
    {/snippet}
  </SettingsRow>
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
