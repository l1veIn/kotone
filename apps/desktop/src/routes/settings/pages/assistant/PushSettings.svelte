<script lang="ts">
  // 「游戏陪玩 → 角色 → 交互 → 推送」（ADR 043 D4）：插件推送时右下角的消息提示（ADR 039 D4）。原来这里的“自动卡片”总开关
  // 去掉了：每条推送在插件页的「设置 → 推送」里单独开关（旧配置里总开关关着的，带卡片的推送算关，读旧写新）。
  // 设置在 config.json `agent.autoCards.popup`（旧配置没有时按开）；写的时候 `enabled` 原样带上（不动旧值）。
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { companionName } from "$lib/stores/companion";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  const config = $derived({ enabled: true, popup: true, ...($settingsStore?.agent?.autoCards ?? {}) });

  async function save(popup: boolean) {
    await patchSettings({ agent: { autoCards: { ...config, popup } } }, popup ? m.companion_pushes_popup_on() : m.companion_pushes_popup_off());
  }
</script>

<SettingsSection title={m.companion_pushes_title()} description={m.companion_pushes_desc({ companion: $companionName })} id="push-settings">
  <div data-testid="push-settings" class="contents">
    <SettingsRow label={m.companion_pushes_popup_label()} description={m.companion_pushes_popup_desc()}>
      {#snippet control()}
        <Switch data-testid="push-popup" aria-label={m.companion_pushes_popup_label()}
          bind:checked={() => config.popup, (checked) => void save(checked)} />
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
